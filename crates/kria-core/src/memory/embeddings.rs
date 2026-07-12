use ort::session::Session;
use ort::value::Tensor;
/// Local sentence embeddings.
///
/// Uses ONNX Runtime (`ort`) for real embedding inference with
/// `all-MiniLM-L6-v2` and the model's real WordPiece tokenizer
/// (`tokenizer.json`, via the `tokenizers` crate). Falls back to
/// deterministic hash-based vectors only if the model OR its tokenizer is
/// unavailable — a real semantic model with a placeholder tokenizer would
/// produce meaningless embeddings, so BOTH are required for the ONNX path.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct EmbeddingModel {
    dim: usize,
    session: Option<Arc<Mutex<Session>>>,
    /// The model's real tokenizer (loaded from `tokenizer.json` next to the
    /// `.onnx` file). `Some` iff the ONNX path is active — the placeholder
    /// hash tokenizer is gone (it produced garbage vocab ids).
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
}

impl EmbeddingModel {
    /// Load the embedding model from an ONNX file + its `tokenizer.json`.
    /// If either is missing, falls back to the hash-based placeholder vectors.
    pub fn load(dim: usize) -> anyhow::Result<Self> {
        // Try to find the ONNX model in standard locations.
        let model_paths = [
            dirs::home_dir()
                .unwrap_or_default()
                .join(".kria/models/embeddings/all-MiniLM-L6-v2.onnx"),
            PathBuf::from("models/embeddings/all-MiniLM-L6-v2.onnx"),
        ];

        for path in &model_paths {
            if !path.exists() {
                continue;
            }
            // The real tokenizer must sit next to the model. Without it the ONNX
            // model would receive garbage token ids, so we treat a missing/invalid
            // tokenizer as "no ONNX" and honestly fall back.
            let tok_path = path
                .parent()
                .map(|p| p.join("tokenizer.json"))
                .filter(|p| p.exists());
            let tokenizer = match tok_path {
                Some(tp) => match tokenizers::Tokenizer::from_file(&tp) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        tracing::warn!(path = %tp.display(), error = %e, "tokenizer.json load failed; hash fallback");
                        None
                    }
                },
                None => {
                    tracing::warn!(path = %path.display(), "ONNX model found but tokenizer.json missing; hash fallback");
                    None
                }
            };
            let Some(tokenizer) = tokenizer else {
                continue;
            };

            match Session::builder()?.commit_from_file(path) {
                Ok(session) => {
                    tracing::info!(path = %path.display(), "embedding model loaded (ONNX + real tokenizer)");
                    return Ok(Self {
                        dim,
                        session: Some(Arc::new(Mutex::new(session))),
                        tokenizer: Some(Arc::new(tokenizer)),
                    });
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to load ONNX model, using fallback");
                }
            }
        }

        tracing::info!(dim, "embedding model initialized (hash-based fallback)");
        Ok(Self {
            dim,
            session: None,
            tokenizer: None,
        })
    }

    /// Generate an embedding vector for the given text.
    pub fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        match (&self.session, &self.tokenizer) {
            (Some(session), Some(tokenizer)) => {
                let mut guard = session
                    .lock()
                    .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;
                self.embed_onnx(&mut guard, tokenizer, text)
            }
            _ => self.embed_fallback(text),
        }
    }

    /// Real ONNX inference for sentence embedding, using the model's real
    /// WordPiece tokenizer + attention-mask-weighted mean pooling (the standard
    /// sentence-transformers pooling for all-MiniLM-L6-v2).
    fn embed_onnx(
        &self,
        session: &mut Session,
        tokenizer: &tokenizers::Tokenizer,
        text: &str,
    ) -> anyhow::Result<Vec<f32>> {
        use ndarray::Array2;

        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenize failed: {e}"))?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&t| t as i64).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        let types: Vec<i64> = encoding.get_type_ids().iter().map(|&t| t as i64).collect();
        let seq_len = ids.len().max(1);

        let input_ids = Array2::from_shape_vec((1, seq_len), ids)?;
        let attention_mask = Array2::from_shape_vec((1, seq_len), mask.clone())?;
        let token_type_ids = Array2::from_shape_vec((1, seq_len), types)?;

        let input_ids_val = Tensor::from_array(input_ids)?;
        let attention_mask_val = Tensor::from_array(attention_mask)?;
        let token_type_ids_val = Tensor::from_array(token_type_ids)?;

        let outputs = session.run(ort::inputs![
            input_ids_val,
            attention_mask_val,
            token_type_ids_val,
        ])?;

        // last_hidden_state: [batch=1, seq_len, hidden_dim]
        let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        let hidden_dim = if shape.len() >= 3 {
            shape[2] as usize
        } else {
            self.dim
        };
        let seq_len_out = if shape.len() >= 3 {
            shape[1] as usize
        } else {
            1
        };

        // Attention-mask-weighted mean pool over the sequence dimension.
        let mut pooled = vec![0.0f32; hidden_dim];
        let mut mask_sum = 0.0f32;
        for s in 0..seq_len_out {
            let m = *mask.get(s).unwrap_or(&1) as f32;
            if m == 0.0 {
                continue;
            }
            mask_sum += m;
            let offset = s * hidden_dim;
            for d in 0..hidden_dim {
                if offset + d < data.len() {
                    pooled[d] += data[offset + d] * m;
                }
            }
        }
        if mask_sum > 0.0 {
            for v in &mut pooled {
                *v /= mask_sum;
            }
        }

        // L2 normalize (cosine-ready).
        let norm: f32 = pooled.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut pooled {
                *v /= norm;
            }
        }

        Ok(pooled)
    }

    /// Deterministic hash-based fallback embedding (no model needed).
    fn embed_fallback(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut vec = vec![0.0f32; self.dim];
        for (i, byte) in text.bytes().enumerate() {
            vec[i % self.dim] += (byte as f32 - 96.0) / 128.0;
        }
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        Ok(vec)
    }

    pub fn dimension(&self) -> usize {
        self.dim
    }

    /// Whether the real ONNX model (with its real tokenizer) is loaded.
    pub fn is_onnx_loaded(&self) -> bool {
        self.session.is_some() && self.tokenizer.is_some()
    }
}
