use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;
use tokenizers::Tokenizer;

use super::turn_gate::{ComputeClass, Operation};

const ENV_ENABLE_ONNX_L0: &str = "KRIA_ENABLE_ONNX_L0";
const ENV_ONNX_MODEL_PATH: &str = "KRIA_ONNX_L0_MODEL_PATH";
const ENV_ONNX_TOKENIZER_PATH: &str = "KRIA_ONNX_L0_TOKENIZER_PATH";
const ENV_ONNX_CORPUS_PATH: &str = "KRIA_ONNX_L0_CORPUS";

const DEFAULT_MODEL_PATH: &str = "~/.kria/models/classifier/model.onnx";
const TOKENIZER_FILENAME: &str = "tokenizer.json";
const WORKSPACE_MODEL_RELATIVE: &str = "../../models/classifier/model.onnx";
const WORKSPACE_TOKENIZER_RELATIVE: &str = "../../models/classifier/tokenizer.json";
const WORKSPACE_CORPUS_RELATIVE: &str = "../../ai-context/onnx_l0_corpus.jsonl";

const MIN_SIMILARITY_FOR_HINT: f32 = 0.20;
const SOFTMAX_TEMPERATURE: f32 = 8.0;

const EMBEDDED_CORPUS_JSONL: &str = r#"
{"text":"generate image of a cyberpunk city at night","operation":"generate_image"}
{"text":"create a logo image for my startup","operation":"generate_image"}
{"text":"draw a watercolor portrait from this prompt","operation":"generate_image"}
{"text":"make an illustration of a mountain village","operation":"generate_image"}
{"text":"render a product photo style hero shot","operation":"generate_image"}
{"text":"generate a wallpaper image in neon style","operation":"generate_image"}
{"text":"what do you remember about my schedule","operation":"retrieve_memory"}
{"text":"recall notes about my gym routine","operation":"retrieve_memory"}
{"text":"memory notes about meeting preferences","operation":"retrieve_memory"}
{"text":"have i told you my food allergies","operation":"retrieve_memory"}
{"text":"from memory tell me my wifi setup","operation":"retrieve_memory"}
{"text":"remembered details about my doctor appointment","operation":"retrieve_memory"}
{"text":"search the web for rust async patterns","operation":"search"}
{"text":"find online latest headlines about ai","operation":"search"}
{"text":"look up internet speed troubleshooting guide","operation":"search"}
{"text":"search news about nvidia drivers","operation":"search"}
{"text":"browse online docs for tokio channels","operation":"search"}
{"text":"check the internet for weather update","operation":"search"}
"#;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnnxHint {
    pub operation: Operation,
    pub compute: ComputeClass,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnnxClassifierStatus {
    Ready,
    Unavailable,
}

struct ClassifierJob {
    text: String,
    response_tx: SyncSender<Option<OnnxHint>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawCorpusEntry {
    text: String,
    operation: String,
}

#[derive(Debug, Clone)]
struct CorpusEntry {
    text: String,
    operation: Operation,
}

#[derive(Debug, Clone)]
struct RuntimeSettings {
    model_path: PathBuf,
    tokenizer_path: PathBuf,
    corpus_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct LabelPrototype {
    operation: Operation,
    compute: ComputeClass,
    embedding: Vec<f32>,
}

struct OnnxRuntime {
    session: Session,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
    prototypes: Vec<LabelPrototype>,
}

/// Optional bounded worker for lightweight L0 intent hints.
///
/// This is intentionally non-authoritative: callers should only consume
/// outputs as hints after deterministic and semantic routes have had priority.
#[derive(Clone)]
pub struct OnnxClassifier {
    tx: SyncSender<ClassifierJob>,
    timeout: Duration,
    status: OnnxClassifierStatus,
}

impl std::fmt::Debug for OnnxClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxClassifier")
            .field("timeout", &self.timeout)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl OnnxClassifier {
    pub fn new(queue_capacity: usize, timeout: Duration) -> Self {
        let (tx, rx) = sync_channel::<ClassifierJob>(queue_capacity.max(1));
        let settings = resolve_runtime_settings();
        let (runtime, status) = OnnxRuntime::load(&settings).map_or_else(
            |error| {
                tracing::warn!(
                    model = %settings.model_path.display(),
                    tokenizer = %settings.tokenizer_path.display(),
                    error = %error,
                    "L0 ONNX classifier disabled: failed to initialize runtime"
                );
                (None, OnnxClassifierStatus::Unavailable)
            },
            |runtime| (Some(runtime), OnnxClassifierStatus::Ready),
        );

        let _ = std::thread::Builder::new()
            .name("kria-onnx-l0-worker".to_string())
            .spawn(move || worker_loop(rx, runtime));

        Self {
            tx,
            timeout,
            status,
        }
    }

    pub fn status(&self) -> OnnxClassifierStatus {
        self.status
    }

    pub fn classify(&self, text: &str) -> Option<OnnxHint> {
        let (response_tx, response_rx) = sync_channel::<Option<OnnxHint>>(1);
        let job = ClassifierJob {
            text: text.to_string(),
            response_tx,
        };

        // Bounded queue: if saturated, skip hinting instead of blocking.
        if self.tx.try_send(job).is_err() {
            tracing::warn!("L0 ONNX classifier queue saturated; skipping hint for this turn");
            return None;
        }

        let hint = response_rx.recv_timeout(self.timeout).ok().flatten();
        if let Some(ref value) = hint {
            tracing::info!(
                operation = ?value.operation,
                compute = ?value.compute,
                confidence = value.confidence,
                "L0 ONNX classifier emitted routing hint"
            );
        }

        hint
    }
}

/// Backward-compatible name used by existing call sites.
///
/// Env var takes priority when explicitly set; otherwise this reads
/// `[classifier].enabled` from runtime config.
pub fn enabled_from_env() -> bool {
    if let Ok(value) = std::env::var(ENV_ENABLE_ONNX_L0) {
        if let Some(parsed) = parse_bool_like(&value) {
            return parsed;
        }
    }

    configured_classifier_from_config()
        .map(|cfg| cfg.enabled)
        .unwrap_or(false)
}

impl OnnxRuntime {
    fn load(settings: &RuntimeSettings) -> anyhow::Result<Self> {
        if !settings.model_path.exists() {
            bail!("model file not found at {}", settings.model_path.display());
        }
        if !settings.tokenizer_path.exists() {
            bail!(
                "tokenizer file not found at {}",
                settings.tokenizer_path.display()
            );
        }

        let session = Session::builder()?
            .commit_from_file(&settings.model_path)
            .with_context(|| format!("loading ONNX model {}", settings.model_path.display()))?;

        let tokenizer = Tokenizer::from_file(&settings.tokenizer_path).map_err(|error| {
            anyhow!(
                "loading tokenizer {} failed: {error}",
                settings.tokenizer_path.display()
            )
        })?;

        let input_names: Vec<String> = session
            .inputs()
            .iter()
            .map(|input| input.name().to_string())
            .collect();

        if input_names.is_empty() {
            bail!("ONNX classifier model has no declared inputs");
        }

        let mut runtime = Self {
            session,
            tokenizer,
            input_names,
            prototypes: Vec::new(),
        };

        let corpus = load_calibration_corpus(settings);
        runtime.prototypes = build_label_prototypes(&mut runtime, &corpus)?;

        tracing::info!(
            model = %settings.model_path.display(),
            tokenizer = %settings.tokenizer_path.display(),
            labels = runtime.prototypes.len(),
            "L0 ONNX classifier initialized"
        );

        Ok(runtime)
    }

    fn classify(&mut self, text: &str) -> anyhow::Result<Option<OnnxHint>> {
        if self.prototypes.is_empty() {
            return Ok(None);
        }

        let mut query = self.embed_text(text)?;
        if !l2_normalize(&mut query) {
            return Ok(None);
        }

        let mut similarities = Vec::with_capacity(self.prototypes.len());
        let mut best_index = 0usize;
        let mut best_similarity = f32::MIN;

        for (index, prototype) in self.prototypes.iter().enumerate() {
            let similarity = dot_product(&query, &prototype.embedding);
            similarities.push(similarity);

            if similarity > best_similarity {
                best_similarity = similarity;
                best_index = index;
            }
        }

        if best_similarity < MIN_SIMILARITY_FOR_HINT {
            return Ok(None);
        }

        let prototype = &self.prototypes[best_index];
        let softmax_confidence = softmax_confidence(&similarities, best_index);
        let similarity_confidence = ((best_similarity + 1.0) * 0.5).clamp(0.0, 1.0);
        let confidence =
            ((softmax_confidence * 0.65) + (similarity_confidence * 0.35)).clamp(0.0, 0.99);

        Ok(Some(OnnxHint {
            operation: prototype.operation,
            compute: prototype.compute,
            confidence,
        }))
    }

    fn embed_text(&mut self, text: &str) -> anyhow::Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| anyhow!("tokenization failed: {error}"))?;

        let mut input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        if input_ids.is_empty() {
            input_ids.push(0);
        }
        let seq_len = input_ids.len();

        let mut attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        if attention_mask.len() != seq_len {
            attention_mask = vec![1; seq_len];
        }

        let mut token_type_ids: Vec<i64> =
            encoding.get_type_ids().iter().map(|&t| t as i64).collect();
        if token_type_ids.len() != seq_len {
            token_type_ids = vec![0; seq_len];
        }

        let outputs = if self.input_names.len() >= 3 {
            let input_ids_tensor = Tensor::from_array(([1usize, seq_len], input_ids))?;
            let attention_mask_tensor =
                Tensor::from_array(([1usize, seq_len], attention_mask.clone()))?;
            let token_type_ids_tensor = Tensor::from_array(([1usize, seq_len], token_type_ids))?;

            self.session.run(ort::inputs![
                self.input_names[0].as_str() => input_ids_tensor,
                self.input_names[1].as_str() => attention_mask_tensor,
                self.input_names[2].as_str() => token_type_ids_tensor,
            ])?
        } else if self.input_names.len() == 2 {
            let input_ids_tensor = Tensor::from_array(([1usize, seq_len], input_ids))?;
            let attention_mask_tensor =
                Tensor::from_array(([1usize, seq_len], attention_mask.clone()))?;

            self.session.run(ort::inputs![
                self.input_names[0].as_str() => input_ids_tensor,
                self.input_names[1].as_str() => attention_mask_tensor,
            ])?
        } else {
            let input_ids_tensor = Tensor::from_array(([1usize, seq_len], input_ids))?;
            self.session
                .run(ort::inputs![self.input_names[0].as_str() => input_ids_tensor])?
        };

        let (shape, data) = if let Some(value) = outputs.get("sentence_embedding") {
            let (shape, data) = value.try_extract_tensor::<f32>()?;
            (shape.to_vec(), data.to_vec())
        } else if let Some(value) = outputs.get("last_hidden_state") {
            let (shape, data) = value.try_extract_tensor::<f32>()?;
            (shape.to_vec(), data.to_vec())
        } else if let Some((_, value)) = outputs.iter().next() {
            let (shape, data) = value.try_extract_tensor::<f32>()?;
            (shape.to_vec(), data.to_vec())
        } else {
            bail!("ONNX classifier did not return tensor outputs");
        };

        extract_embedding(&shape, &data, &attention_mask)
    }
}

fn worker_loop(rx: Receiver<ClassifierJob>, mut runtime: Option<OnnxRuntime>) {
    while let Ok(job) = rx.recv() {
        let hint = if let Some(runtime) = runtime.as_mut() {
            match runtime.classify(&job.text) {
                Ok(value) => value,
                Err(error) => {
                    tracing::debug!(error = %error, "L0 ONNX classifier inference failed");
                    None
                }
            }
        } else {
            None
        };

        let _ = job.response_tx.send(hint);
    }
}

fn build_label_prototypes(
    runtime: &mut OnnxRuntime,
    corpus: &[CorpusEntry],
) -> anyhow::Result<Vec<LabelPrototype>> {
    let mut image_vectors = Vec::new();
    let mut memory_vectors = Vec::new();
    let mut search_vectors = Vec::new();

    for entry in corpus {
        let embedding = runtime
            .embed_text(&entry.text)
            .with_context(|| format!("embedding calibration sample: {}", entry.text))?;

        match entry.operation {
            Operation::GenerateImage => image_vectors.push(embedding),
            Operation::RetrieveMemory => memory_vectors.push(embedding),
            Operation::Search => search_vectors.push(embedding),
            _ => {}
        }
    }

    let mut prototypes = Vec::new();

    if let Some(embedding) = centroid_from_vectors(&image_vectors) {
        prototypes.push(LabelPrototype {
            operation: Operation::GenerateImage,
            compute: ComputeClass::ImageGpu,
            embedding,
        });
    }

    if let Some(embedding) = centroid_from_vectors(&memory_vectors) {
        prototypes.push(LabelPrototype {
            operation: Operation::RetrieveMemory,
            compute: ComputeClass::ToolOnly,
            embedding,
        });
    }

    if let Some(embedding) = centroid_from_vectors(&search_vectors) {
        prototypes.push(LabelPrototype {
            operation: Operation::Search,
            compute: ComputeClass::ToolOnly,
            embedding,
        });
    }

    if prototypes.is_empty() {
        bail!("no valid calibration samples for supported operations");
    }

    Ok(prototypes)
}

fn centroid_from_vectors(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    if vectors.is_empty() {
        return None;
    }

    let dimension = vectors.first()?.len();
    if dimension == 0 {
        return None;
    }

    let mut centroid = vec![0.0f32; dimension];
    let mut valid_count = 0usize;

    for vector in vectors {
        if vector.len() != dimension {
            continue;
        }
        for (dst, src) in centroid.iter_mut().zip(vector.iter()) {
            *dst += *src;
        }
        valid_count += 1;
    }

    if valid_count == 0 {
        return None;
    }

    let scale = 1.0 / (valid_count as f32);
    for value in &mut centroid {
        *value *= scale;
    }

    if !l2_normalize(&mut centroid) {
        return None;
    }

    Some(centroid)
}

fn extract_embedding(
    shape: &[i64],
    data: &[f32],
    attention_mask: &[i64],
) -> anyhow::Result<Vec<f32>> {
    match shape.len() {
        // [batch, seq, hidden]
        3 => {
            let seq_len = shape[1].max(0) as usize;
            let hidden = shape[2].max(0) as usize;
            if hidden == 0 {
                bail!("classifier output hidden dimension is zero");
            }

            let mut pooled = vec![0.0f32; hidden];
            let mut used_tokens = 0usize;

            for token_idx in 0..seq_len {
                let include = attention_mask.get(token_idx).copied().unwrap_or(1) > 0;
                if !include {
                    continue;
                }

                let offset = token_idx * hidden;
                if offset + hidden > data.len() {
                    break;
                }

                for dim in 0..hidden {
                    pooled[dim] += data[offset + dim];
                }
                used_tokens += 1;
            }

            if used_tokens == 0 {
                bail!("classifier output produced no active tokens");
            }

            let scale = 1.0 / (used_tokens as f32);
            for value in &mut pooled {
                *value *= scale;
            }

            if !l2_normalize(&mut pooled) {
                bail!("classifier embedding normalization failed");
            }

            Ok(pooled)
        }
        // [batch, hidden]
        2 => {
            let hidden = shape[1].max(0) as usize;
            if hidden == 0 || data.len() < hidden {
                bail!("classifier output shape incompatible with [batch, hidden]");
            }

            let mut embedding = data[..hidden].to_vec();
            if !l2_normalize(&mut embedding) {
                bail!("classifier embedding normalization failed");
            }
            Ok(embedding)
        }
        // [hidden]
        1 => {
            if data.is_empty() {
                bail!("classifier output tensor is empty");
            }
            let mut embedding = data.to_vec();
            if !l2_normalize(&mut embedding) {
                bail!("classifier embedding normalization failed");
            }
            Ok(embedding)
        }
        _ => bail!("unsupported classifier output rank: {}", shape.len()),
    }
}

fn load_calibration_corpus(settings: &RuntimeSettings) -> Vec<CorpusEntry> {
    if let Some(path) = settings.corpus_path.as_ref() {
        if let Some(entries) = read_corpus_file(path) {
            return entries;
        }
    }

    let workspace_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(WORKSPACE_CORPUS_RELATIVE);
    if let Some(entries) = read_corpus_file(&workspace_path) {
        return entries;
    }

    parse_corpus_jsonl(EMBEDDED_CORPUS_JSONL)
}

fn read_corpus_file(path: &Path) -> Option<Vec<CorpusEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    let entries = parse_corpus_jsonl(&text);
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

fn parse_corpus_jsonl(text: &str) -> Vec<CorpusEntry> {
    let mut entries = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Ok(raw) = serde_json::from_str::<RawCorpusEntry>(trimmed) else {
            continue;
        };

        let Some(operation) = parse_operation_label(&raw.operation) else {
            continue;
        };

        let entry_text = raw.text.trim();
        if entry_text.is_empty() {
            continue;
        }

        entries.push(CorpusEntry {
            text: entry_text.to_string(),
            operation,
        });
    }

    entries
}

fn parse_operation_label(label: &str) -> Option<Operation> {
    let normalized = label.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "generate_image" | "image_generation" | "image" => Some(Operation::GenerateImage),
        "retrieve_memory" | "memory" | "recall" => Some(Operation::RetrieveMemory),
        "search" | "web_search" => Some(Operation::Search),
        _ => None,
    }
}

fn resolve_runtime_settings() -> RuntimeSettings {
    let config = configured_classifier_from_config();

    let configured_model = std::env::var(ENV_ONNX_MODEL_PATH)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            config
                .as_ref()
                .map(|cfg| cfg.model_path.clone())
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_MODEL_PATH.to_string());

    let mut model_candidates = vec![expand_home(&configured_model)];
    push_unique_path(&mut model_candidates, workspace_model_path());
    push_unique_path(&mut model_candidates, expand_home(DEFAULT_MODEL_PATH));
    let model_path = select_existing_or_first(model_candidates);

    let mut tokenizer_candidates = Vec::new();
    if let Some(path) = std::env::var(ENV_ONNX_TOKENIZER_PATH)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand_home(&value))
    {
        push_unique_path(&mut tokenizer_candidates, path);
    }

    let model_dir = model_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    push_unique_path(
        &mut tokenizer_candidates,
        model_dir.join(TOKENIZER_FILENAME),
    );
    push_unique_path(&mut tokenizer_candidates, workspace_tokenizer_path());

    let tokenizer_path = select_existing_or_first(tokenizer_candidates);

    let corpus_path = std::env::var(ENV_ONNX_CORPUS_PATH)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(WORKSPACE_CORPUS_RELATIVE);
            if path.exists() {
                Some(path)
            } else {
                None
            }
        });

    RuntimeSettings {
        model_path,
        tokenizer_path,
        corpus_path,
    }
}

fn configured_classifier_from_config() -> Option<crate::config::ClassifierConfig> {
    crate::config::KriaConfig::load(None)
        .ok()
        .map(|config| config.classifier)
}

fn expand_home(raw_path: &str) -> PathBuf {
    if raw_path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw_path));
    }

    if let Some(stripped) = raw_path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw_path)
}

fn workspace_model_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(WORKSPACE_MODEL_RELATIVE)
}

fn workspace_tokenizer_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(WORKSPACE_TOKENIZER_RELATIVE)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

fn select_existing_or_first(paths: Vec<PathBuf>) -> PathBuf {
    if let Some(existing) = paths.iter().find(|path| path.exists()) {
        return existing.clone();
    }

    paths
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODEL_PATH))
}

fn parse_bool_like(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn l2_normalize(values: &mut [f32]) -> bool {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return false;
    }

    for value in values {
        *value /= norm;
    }

    true
}

fn dot_product(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| l * r)
        .sum::<f32>()
}

fn softmax_confidence(similarities: &[f32], best_index: usize) -> f32 {
    if similarities.is_empty() || best_index >= similarities.len() {
        return 0.0;
    }

    let scaled: Vec<f32> = similarities
        .iter()
        .map(|value| (value * SOFTMAX_TEMPERATURE).exp())
        .collect();
    let denom = scaled.iter().sum::<f32>();
    if denom <= f32::EPSILON {
        return 0.0;
    }

    (scaled[best_index] / denom).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_operation_label_supports_aliases() {
        assert_eq!(
            parse_operation_label("image_generation"),
            Some(Operation::GenerateImage)
        );
        assert_eq!(
            parse_operation_label("retrieve_memory"),
            Some(Operation::RetrieveMemory)
        );
        assert_eq!(parse_operation_label("web_search"), Some(Operation::Search));
        assert_eq!(parse_operation_label("unknown"), None);
    }

    #[test]
    fn parse_corpus_jsonl_skips_invalid_rows() {
        let parsed = parse_corpus_jsonl(
            r#"
{"text":"search web docs","operation":"search"}
{"text":"","operation":"search"}
{"text":"broken"}
{"text":"remember this","operation":"retrieve_memory"}
"#,
        );

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].operation, Operation::Search);
        assert_eq!(parsed[1].operation, Operation::RetrieveMemory);
    }

    #[test]
    fn parse_bool_like_handles_common_values() {
        assert_eq!(parse_bool_like("true"), Some(true));
        assert_eq!(parse_bool_like("ON"), Some(true));
        assert_eq!(parse_bool_like("0"), Some(false));
        assert_eq!(parse_bool_like("maybe"), None);
    }
}
