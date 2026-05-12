//! Visual Reasoning Engine - RFC 008 Phase 4
//!
//! Implements bounded visual reasoning for handling novel UI elements.
//! Per RFC 008 Section 4: "Visual reasoning is scoped, deterministic, and bounded"

use crate::tools::vision_automation::OmniElement;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ============================================================================
// Section 1: EvidenceWrapper Trust Boundary (RFC 008 Section 4.2)
// ============================================================================

/// RFC 008: Evidence wrapper for cognitive poisoning defense.
/// Per RFC 008: "All OCR text wrapped and treated as untrusted"
#[derive(Debug, Clone)]
pub struct EvidenceWrapper {
    /// The raw OCR text (untrusted)
    pub raw_text: String,
    /// Whether text was truncated
    pub was_truncated: bool,
    /// Confidence score from OCR engine (0.0-1.0)
    pub ocr_confidence: f32,
    /// Source of the evidence
    pub source: EvidenceSource,
    /// Timestamp of capture (not serialized)
    pub captured_at: Instant,
}

/// Source of visual evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSource {
    /// OCR text extraction
    Ocr,
    /// Visual feature detection (icon recognition)
    VisualFeature,
    /// Predefined library lookup
    Library,
}

impl EvidenceWrapper {
    /// Create new evidence wrapper from OCR text.
    /// Per RFC 008: "Sentence-aware truncation preserves negation context"
    pub fn from_ocr(text: &str, confidence: f32) -> Self {
        // Apply sentence-aware truncation (preserve negation context)
        let (truncated, was_truncated) = Self::sentence_aware_truncate(text, 100);
        
        Self {
            raw_text: truncated,
            was_truncated,
            ocr_confidence: confidence,
            source: EvidenceSource::Ocr,
            captured_at: Instant::now(),
        }
    }
    
    /// Sentence-aware truncation preserving negation context.
    /// Per RFC 008: "Do NOT delete" vs "Delete" - negation changes meaning
    fn sentence_aware_truncate(text: &str, max_chars: usize) -> (String, bool) {
        if text.len() <= max_chars {
            return (text.to_string(), false);
        }
        
        // Find sentence boundary before max_chars
        let trunc_point = text.chars().take(max_chars).collect::<String>();
        
        // Look for sentence-ending punctuation
        if let Some(last_sentence_end) = trunc_point.rfind(|c| c == '.' || c == '!' || c == '?') {
            // End at sentence boundary + 1 (include the punctuation)
            let result: String = text.chars().take(last_sentence_end + 1).collect();
            (result, true)
        } else if let Some(last_space) = trunc_point.rfind(' ') {
            // Find last negation modifier before truncation point
            let negation_keywords = ["not", "don't", "doesn't", "can't", "won't", "never"];
            let before_truncation = &text[..last_space];
            
            // Check if we're cutting after a negation
            let has_negation_before = negation_keywords.iter().any(|&kw| {
                before_truncation.to_lowercase().contains(&format!(" {} ", kw))
            });
            
            if has_negation_before {
                // Extend to next sentence boundary or safe word
                if let Some(next_sentence) = text[last_space..].find(|c| c == '.' || c == '!') {
                    let extended = text.chars().take(last_space + next_sentence + 1).collect();
                    return (extended, true);
                }
            }
            
            // Word boundary truncation
            (text[..last_space].to_string(), true)
        } else {
            // Hard truncate at max_chars
            (trunc_point, true)
        }
    }
    
    /// Get trust level for this evidence.
    /// OCR is always untrusted per RFC 008
    pub fn trust_level(&self) -> TrustLevel {
        match self.source {
            EvidenceSource::Ocr => TrustLevel::Untrusted,
            EvidenceSource::VisualFeature => TrustLevel::Low,
            EvidenceSource::Library => TrustLevel::High,
        }
    }
    
    /// Check if evidence contains destructive action keywords.
    /// Per RFC 008: "OCR action-verb heuristic reduces confidence"
    pub fn contains_destructive_keywords(&self) -> bool {
        let destructive_keywords = [
            "delete", "remove", "destroy", "format", "wipe", "clear all",
            "empty trash", "permanently delete", "uninstall",
        ];
        
        let lower = self.raw_text.to_lowercase();
        destructive_keywords.iter().any(|&kw| lower.contains(kw))
    }
}

/// Trust levels for evidence sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Fully trusted (library definitions)
    High,
    /// Moderate trust (visual features)
    Low,
    /// Untrusted (OCR text)
    Untrusted,
}

// ============================================================================
// Section 2: Vision ↔ OCR Contradiction Detection (RFC 008 Section 4.2)
// ============================================================================

/// RFC 008: Contradiction between visual and OCR evidence.
#[derive(Debug, Clone)]
pub struct VisualOcrContradiction {
    /// Element ID where contradiction occurred
    pub element_id: String,
    /// Visual evidence (icon/feature detected)
    pub visual_evidence: String,
    /// OCR evidence (text detected)
    pub ocr_evidence: String,
    /// Type of contradiction
    pub contradiction_type: ContradictionType,
    /// Confidence override: forces 0.0
    pub forces_zero_confidence: bool,
}

/// Types of contradictions between vision and OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContradictionType {
    /// Icon says one thing, OCR says opposite (e.g., trash icon + "Save")
    SemanticMismatch,
    /// Visual type differs from OCR interpretation
    TypeMismatch,
    /// Destructive icon with benign OCR
    DestructiveIconBenignText,
    /// Benign icon with destructive OCR
    BenignIconDestructiveText,
}

/// RFC 008: Contradiction detector.
/// Per RFC 008: "Contradictions force confidence to 0.0 regardless of chain"
pub struct ContradictionDetector;

impl ContradictionDetector {
    /// Detect contradiction between visual icon and OCR text.
    /// Per RFC 008: "If icon contradicts OCR, force confidence to 0.0"
    pub fn detect(
        element: &OmniElement,
        _visual_semantic: &str,
        ocr_text: &EvidenceWrapper,
        icon_library: &SemanticIconLibrary,
    ) -> Option<VisualOcrContradiction> {
        // Get expected semantic from icon library
        let visual_signature = Self::extract_visual_signature(element);
        
        if let Some(icon_semantic) = icon_library.lookup_semantic(&visual_signature) {
            // Check for semantic mismatch
            let ocr_lower = ocr_text.raw_text.to_lowercase();
            
            // Destructive icon with benign text (e.g., trash + "Save")
            if icon_semantic.is_destructive && !Self::is_destructive_text(&ocr_lower) {
                return Some(VisualOcrContradiction {
                    element_id: element.id.clone(),
                    visual_evidence: icon_semantic.semantic_meaning.clone(),
                    ocr_evidence: ocr_text.raw_text.clone(),
                    contradiction_type: ContradictionType::DestructiveIconBenignText,
                    forces_zero_confidence: true,
                });
            }
            
            // Benign icon with destructive text
            if !icon_semantic.is_destructive && Self::is_destructive_text(&ocr_lower) {
                return Some(VisualOcrContradiction {
                    element_id: element.id.clone(),
                    visual_evidence: icon_semantic.semantic_meaning.clone(),
                    ocr_evidence: ocr_text.raw_text.clone(),
                    contradiction_type: ContradictionType::BenignIconDestructiveText,
                    forces_zero_confidence: true,
                });
            }
            
            // General semantic mismatch
            if !Self::semantics_match(&icon_semantic.semantic_meaning, &ocr_lower) {
                return Some(VisualOcrContradiction {
                    element_id: element.id.clone(),
                    visual_evidence: icon_semantic.semantic_meaning.clone(),
                    ocr_evidence: ocr_text.raw_text.clone(),
                    contradiction_type: ContradictionType::SemanticMismatch,
                    forces_zero_confidence: true,
                });
            }
        }
        
        None
    }
    
    /// Extract visual signature from element (scaffolding).
    fn extract_visual_signature(element: &OmniElement) -> VisualSignature {
        // In production: would use actual visual features (pHash, CNN embeddings)
        // For now, use element type as proxy
        VisualSignature {
            feature_hash: element.visual_hash.clone(),
            element_type: element.element_type.clone(),
            bbox: element.bbox,
        }
    }
    
    /// Check if OCR text indicates destructive action.
    fn is_destructive_text(text: &str) -> bool {
        let destructive = ["delete", "remove", "destroy", "trash", "wipe", "format"];
        destructive.iter().any(|&kw| text.contains(kw))
    }
    
    /// Check if visual semantics match OCR text.
    fn semantics_match(visual: &str, ocr: &str) -> bool {
        let visual_lower = visual.to_lowercase();
        // Simple overlap check - in production would use embeddings
        visual_lower.contains(ocr) || ocr.contains(&visual_lower)
    }
}

// ============================================================================
// Section 3: Semantic Confidence Propagation (RFC 008 Section 4.2)
// ============================================================================

/// RFC 008: Confidence chain components.
/// Per RFC 008: "Multiplicative propagation: Prereq * Visual * Exploration"
#[derive(Debug, Clone, Default)]
pub struct ConfidenceChain {
    /// Prerequisite check confidence (0.0-1.0)
    pub prerequisite_confidence: f32,
    /// Visual reasoning confidence (0.0-1.0)
    pub visual_reasoning_confidence: f32,
    /// Safe exploration confidence (0.0-1.0)
    pub exploration_confidence: f32,
}

impl ConfidenceChain {
    /// Calculate final confidence via multiplicative propagation.
    /// Per RFC 008: "Multiplicative propagation ensures uncertainty compounds"
    pub fn calculate_final(&self) -> f32 {
        self.prerequisite_confidence 
            * self.visual_reasoning_confidence 
            * self.exploration_confidence
    }
    
    /// Apply lower bound clamp per RFC 008.
    pub fn apply_lower_bound(confidence: f32) -> f32 {
        const LOWER_BOUND: f32 = 0.15;
        confidence.max(LOWER_BOUND)
    }
}

/// RFC 008: Novel element confidence ceiling.
/// Per RFC 008: "For all novel UI elements, cap confidence at 0.90"
pub const NOVEL_ELEMENT_CONFIDENCE_CEILING: f32 = 0.90;

/// Apply confidence ceiling for novel elements.
pub fn apply_novel_element_ceiling(
    calculated_confidence: f32,
    is_known_element: bool,
) -> f32 {
    if !is_known_element {
        // Novel element: cap at 0.90 regardless of reasoning
        let capped = calculated_confidence.min(NOVEL_ELEMENT_CONFIDENCE_CEILING);
        
        tracing::info!(
            "Novel element confidence capped: {} -> {}",
            calculated_confidence, capped
        );
        
        capped
    } else {
        // Known element: use calculated confidence
        calculated_confidence
    }
}

// ============================================================================
// Section 4: Semantic Icon Library (RFC 008 Section 4.4)
// ============================================================================

/// Visual signature for icon matching.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct VisualSignature {
    /// Perceptual hash of the icon
    pub feature_hash: String,
    /// Element type classification
    pub element_type: String,
    /// Bounding box location (for position-based matching)
    pub bbox: [i32; 4],
}

/// Semantic meaning of an icon.
#[derive(Debug, Clone)]
pub struct IconSemantics {
    /// Human-readable semantic meaning
    pub semantic_meaning: String,
    /// Action verb associated with this icon
    pub action_verb: String,
    /// Whether this icon represents destructive action
    pub is_destructive: bool,
    /// Minimum confidence threshold for recognition
    pub min_confidence: f32,
}

/// RFC 008: Semantic Icon Library.
/// Per RFC 008 Section 4.4: "Library maps visual patterns to functions"
pub struct SemanticIconLibrary {
    /// Map of visual signature -> semantic meaning
    known_icons: HashMap<String, IconSemantics>,
    /// User-taught contradiction exceptions
    contradiction_exceptions: Vec<ContradictionException>,
}

/// User-taught exception for valid contradictions.
#[derive(Debug, Clone)]
pub struct ContradictionException {
    /// Exception ID
    pub exception_id: String,
    /// App identity where exception applies
    pub app_name: String,
    /// Visual signature pattern
    pub visual_pattern: String,
    /// Expected semantics (what vision thinks)
    pub expected_semantics: String,
    /// Actual semantics (what user confirms)
    pub actual_semantics: String,
    /// When exception was created (not serialized)
    pub created_at: Instant,
    /// Optional expiration (not serialized)
    pub expires_at: Option<Instant>,
}

impl SemanticIconLibrary {
    /// Create library with default icon mappings.
    pub fn new() -> Self {
        let mut library = Self {
            known_icons: HashMap::new(),
            contradiction_exceptions: Vec::new(),
        };
        library.register_default_icons();
        library
    }
    
    /// Register default icon semantics per RFC 008.
    fn register_default_icons(&mut self) {
        // Plus sign = Create/Add
        self.register("plus_sign", IconSemantics {
            semantic_meaning: "create or add new item".to_string(),
            action_verb: "create".to_string(),
            is_destructive: false,
            min_confidence: 0.85,
        });
        
        // Trash can = Delete
        self.register("trash_icon", IconSemantics {
            semantic_meaning: "delete or remove item".to_string(),
            action_verb: "delete".to_string(),
            is_destructive: true,
            min_confidence: 0.90,
        });
        
        // Magnifying glass = Search
        self.register("magnifying_glass", IconSemantics {
            semantic_meaning: "search or find".to_string(),
            action_verb: "search".to_string(),
            is_destructive: false,
            min_confidence: 0.85,
        });
        
        // Floppy disk = Save
        self.register("floppy_disk", IconSemantics {
            semantic_meaning: "save changes".to_string(),
            action_verb: "save".to_string(),
            is_destructive: false,
            min_confidence: 0.90,
        });
        
        // Pencil = Edit
        self.register("pencil_icon", IconSemantics {
            semantic_meaning: "edit or modify".to_string(),
            action_verb: "edit".to_string(),
            is_destructive: false,
            min_confidence: 0.85,
        });
        
        // Gear/Settings = Configure
        self.register("gear_icon", IconSemantics {
            semantic_meaning: "open settings or configuration".to_string(),
            action_verb: "configure".to_string(),
            is_destructive: false,
            min_confidence: 0.85,
        });
        
        // X/Cross = Close/Cancel
        self.register("x_icon", IconSemantics {
            semantic_meaning: "close dialog or cancel action".to_string(),
            action_verb: "close".to_string(),
            is_destructive: false,
            min_confidence: 0.80,
        });
        
        // Checkmark = Confirm/OK
        self.register("checkmark_icon", IconSemantics {
            semantic_meaning: "confirm or accept".to_string(),
            action_verb: "confirm".to_string(),
            is_destructive: false,
            min_confidence: 0.85,
        });
    }
    
    /// Register icon semantic mapping.
    pub fn register(&mut self, pattern: &str, semantics: IconSemantics) {
        self.known_icons.insert(pattern.to_string(), semantics);
    }
    
    /// Lookup semantic meaning for visual signature.
    pub fn lookup_semantic(&self, signature: &VisualSignature) -> Option<&IconSemantics> {
        // Exact match first
        if let Some(semantic) = self.known_icons.get(&signature.feature_hash) {
            return Some(semantic);
        }
        
        // Pattern match on element type (scaffolding)
        match signature.element_type.as_str() {
            "button" => self.known_icons.get("plus_sign"),
            "icon" => self.known_icons.get("magnifying_glass"),
            _ => None,
        }
    }
    
    /// Check if element is known in library.
    pub fn contains(&self, signature: &VisualSignature) -> bool {
        self.known_icons.contains_key(&signature.feature_hash)
            || self.known_icons.contains_key(&signature.element_type)
    }
    
    /// Add contradiction exception (user teach-in).
    pub fn add_exception(&mut self, exception: ContradictionException) {
        self.contradiction_exceptions.push(exception);
    }
    
    /// Find matching exception for contradiction.
    pub fn find_exception(
        &self,
        app_name: &str,
        visual_pattern: &str,
    ) -> Option<&ContradictionException> {
        self.contradiction_exceptions.iter().find(|e| {
            e.app_name == app_name && e.visual_pattern == visual_pattern
        })
    }
}

impl Default for SemanticIconLibrary {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Section 5: Safe Exploration Mode (RFC 008 Section 4.3)
// ============================================================================

/// RFC 008: Exploration policy tiers.
/// Per RFC 008: "Safe, Restricted, Forbidden based on application context"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorationTier {
    /// Safe exploration: can interact, hover, observe
    Safe,
    /// Restricted exploration: hover-only, no state changes
    Restricted,
    /// Forbidden: no exploration allowed
    Forbidden,
}

/// Application context for tier determination.
#[derive(Debug, Clone)]
pub struct AppContext {
    /// Application name/class
    pub app_name: String,
    /// Window title
    pub window_title: String,
    /// Current URL (for browsers)
    pub current_url: Option<String>,
    /// Is payment page
    pub is_payment_page: bool,
}

/// RFC 008: Safe Explorer with policy tiers.
/// Per RFC 008 Section 4.3: "Exploration policy tiers enforced by application context"
pub struct SafeExplorer {
    /// Current exploration tier
    current_tier: ExplorationTier,
    /// Semantic icon library for visual reasoning
    icon_library: SemanticIconLibrary,
    /// Explored elements cache (to avoid re-exploration)
    explored_elements: HashMap<String, ExplorationResult>,
    /// Maximum exploration actions per element
    #[allow(dead_code)]
    max_exploration_actions: u32,
}

/// Result of exploring an element.
#[derive(Debug, Clone)]
pub struct ExplorationResult {
    /// Element ID
    pub element_id: String,
    /// Tooltip text captured
    pub tooltip_text: Option<String>,
    /// Visual semantics inferred
    pub visual_semantics: Option<String>,
    /// Confidence in exploration result
    pub confidence: f32,
    /// Timestamp (not serialized)
    pub explored_at: Instant,
}

impl SafeExplorer {
    /// Create new safe explorer.
    pub fn new() -> Self {
        Self {
            current_tier: ExplorationTier::Safe,
            icon_library: SemanticIconLibrary::new(),
            explored_elements: HashMap::new(),
            max_exploration_actions: 3,
        }
    }
    
    /// Set exploration tier based on app context.
    /// Per RFC 008: "Runtime-sensitive overrides: browser payment detection"
    pub fn set_tier_for_context(&mut self, context: &AppContext) {
        let new_tier = self.determine_tier(context);
        
        if new_tier != self.current_tier {
            tracing::info!(
                "Exploration tier changed: {:?} -> {:?} for {}",
                self.current_tier, new_tier, context.app_name
            );
            self.current_tier = new_tier;
        }
    }
    
    /// Determine tier from app context.
    fn determine_tier(&self, context: &AppContext) -> ExplorationTier {
        // Payment/checkout pages: Forbidden
        if context.is_payment_page {
            return ExplorationTier::Forbidden;
        }
        
        // Check URL for payment keywords
        if let Some(url) = &context.current_url {
            let url_lower = url.to_lowercase();
            let payment_keywords = ["checkout", "payment", "billing", "credit-card", "paypal"];
            if payment_keywords.iter().any(|kw| url_lower.contains(kw)) {
                return ExplorationTier::Forbidden;
            }
        }
        
        // Known safe applications
        let safe_apps = ["gedit", "notepad", "calculator", "files"];
        if safe_apps.iter().any(|&app| context.app_name.to_lowercase().contains(app)) {
            return ExplorationTier::Safe;
        }
        
        // Known restricted applications
        let restricted_apps = ["terminal", "vscode", "settings"];
        if restricted_apps.iter().any(|&app| context.app_name.to_lowercase().contains(app)) {
            return ExplorationTier::Restricted;
        }
        
        // Default: Restricted for unknown
        ExplorationTier::Restricted
    }
    
    /// Check if exploration is allowed.
    pub fn can_explore(&self) -> bool {
        match self.current_tier {
            ExplorationTier::Safe | ExplorationTier::Restricted => true,
            ExplorationTier::Forbidden => false,
        }
    }
    
    /// Check if hover is allowed.
    pub fn can_hover(&self) -> bool {
        match self.current_tier {
            ExplorationTier::Safe | ExplorationTier::Restricted => true,
            ExplorationTier::Forbidden => false,
        }
    }
    
    /// Check if click is allowed.
    pub fn can_click(&self) -> bool {
        match self.current_tier {
            ExplorationTier::Safe => true,
            ExplorationTier::Restricted | ExplorationTier::Forbidden => false,
        }
    }
    
    /// Hover over element and capture tooltip.
    /// Per RFC 008: "Hover-only for uncertain elements"
    pub async fn hover_element(&mut self, element: &OmniElement) -> Option<ExplorationResult> {
        if !self.can_hover() {
            tracing::warn!("Hover not allowed in {:?} tier", self.current_tier);
            return None;
        }
        
        // Check if already explored
        if let Some(result) = self.explored_elements.get(&element.id) {
            // Reuse if recent (< 30 seconds)
            if result.explored_at.elapsed() < Duration::from_secs(30) {
                return Some(result.clone());
            }
        }
        
        // Scaffolding: In production, would:
        // 1. Move mouse to element
        // 2. Wait for tooltip
        // 3. Capture tooltip region
        // 4. OCR tooltip text
        
        let result = ExplorationResult {
            element_id: element.id.clone(),
            tooltip_text: Some(format!("Tooltip for {}", element.id)),
            visual_semantics: self.infer_visual_semantics(element),
            confidence: 0.75,
            explored_at: Instant::now(),
        };
        
        self.explored_elements.insert(element.id.clone(), result.clone());
        
        Some(result)
    }
    
    /// Infer visual semantics from element.
    fn infer_visual_semantics(&self, element: &OmniElement) -> Option<String> {
        let signature = VisualSignature {
            feature_hash: element.visual_hash.clone(),
            element_type: element.element_type.clone(),
            bbox: element.bbox,
        };
        
        self.icon_library.lookup_semantic(&signature)
            .map(|s| s.semantic_meaning.clone())
    }
    
    /// Get current tier.
    pub fn current_tier(&self) -> ExplorationTier {
        self.current_tier
    }
}

impl Default for SafeExplorer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Section 6: Content Generator (RFC 008 Semantic Intent Parsing)
// ============================================================================

/// RFC 008: Content type classification for intent parsing.
/// Per RFC 008 Section 4.2: "Distinguish Generated Content from Literal Content"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// Content that requires generation (code, algorithms, essays)
    Generated,
    /// Content that should be used as-is (names, credentials, specific phrases)
    Literal,
}

/// RFC 008: Content generation result with provenance metadata.
#[derive(Debug, Clone)]
pub struct GeneratedContent {
    /// The generated content
    pub content: String,
    /// Content type classification
    pub content_type: ContentType,
    /// Source of generation
    pub generation_source: ContentSource,
    /// Timestamp of generation
    pub generated_at: Instant,
    /// Confidence in generation
    pub confidence: f32,
}

/// Source of content generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSource {
    /// Generated by internal reasoning
    AgentGenerated,
    /// Taken literally from user prompt
    UserProvided,
    /// From OCR evidence (untrusted)
    OcrExtraction,
}

/// RFC 008: Content generator for semantic intent parsing.
/// Per RFC 008: "Evaluates if intent requires Generated Content vs Literal Content"
pub struct ContentGenerator;

impl ContentGenerator {
    /// Classify user intent as Generated or Literal content.
    /// Per RFC 008: "Code/Math/Essays = Generated; Names/Credentials = Literal"
    ///
    /// Priority Rules (RFC 008 §4.2):
    ///   1. If intent STARTS with "type "/"enter "/"input " → always Literal (explicit command)
    ///   2. If intent contains generation keywords → Generated (even if "type" appears mid-sentence)
    ///   3. If intent contains ONLY literal markers (no generation keywords) → Literal
    ///   4. Default → Literal (safe fallback)
    pub fn classify_content_type(user_intent: &str) -> ContentType {
        let lower = user_intent.to_lowercase();
        
        // Keywords indicating generated content (requires reasoning)
        let generated_keywords = [
            "code", "program", "script", "function", "algorithm",
            "implement", "write a", "generate", "create a", "solve",
            "fibonacci", "factorial", "sort", "search", "algorithm",
            "calculate", "compute", "equation", "formula", "math",
            "essay", "article", "summary", "analysis", "report",
            "python", "javascript", "rust", "java", "cpp", "c++",
            "data structure", "class", "method", "api", "library",
            "sequence", "prime", "binary", "recursive", "loop",
        ];
        
        // Keywords indicating literal content (use as-is)
        let literal_keywords = [
            "username", "password",
            "login", "credential", "name:", "email:", "phone:",
            "address:", "specific", "exactly", "literally",
            "my name is", "i am ", "call me", "signed",
        ];
        
        // Check for generated content indicators
        let has_generated_keyword = generated_keywords.iter().any(|kw| lower.contains(kw));
        let has_literal_marker = literal_keywords.iter().any(|kw| lower.contains(kw));
        
        // Rule 1: Explicit typing command at start → always Literal
        // "type Hello World" or "enter my name" are direct commands
        let is_explicit_typing_command = lower.starts_with("type ")
            || lower.starts_with("enter ")
            || lower.starts_with("input ");
        
        if is_explicit_typing_command && !has_generated_keyword {
            tracing::debug!(
                "classify_content_type: '{}' → Literal (explicit typing command, no generation keywords)",
                user_intent
            );
            return ContentType::Literal;
        }
        
        // Rule 2: Generation keywords present → Generated wins
        // Mid-sentence "type" (e.g., "open gedit and type a fibonacci program")
        // is a verb describing action, NOT a literal typing command
        if has_generated_keyword {
            tracing::debug!(
                "classify_content_type: '{}' → Generated (generation keywords found, has_literal_marker={})",
                user_intent, has_literal_marker
            );
            return ContentType::Generated;
        }
        
        // Rule 3: Only literal markers, no generation keywords → Literal
        if has_literal_marker {
            tracing::debug!(
                "classify_content_type: '{}' → Literal (literal markers only)",
                user_intent
            );
            return ContentType::Literal;
        }
        
        // Rule 4: Default to literal for safety
        tracing::debug!(
            "classify_content_type: '{}' → Literal (default fallback)",
            user_intent
        );
        ContentType::Literal
    }
    
    /// Generate content based on intent.
    /// Per RFC 008: "Invoke internal reasoning for Generated Content"
    pub fn generate_content(user_intent: &str) -> GeneratedContent {
        let content_type = Self::classify_content_type(user_intent);
        
        match content_type {
            ContentType::Generated => {
                // Scaffolding: In production, would use LLM to generate content
                // For now, use pattern-based generation for common cases
                let generated = Self::pattern_based_generation(user_intent);
                
                GeneratedContent {
                    content: generated,
                    content_type: ContentType::Generated,
                    generation_source: ContentSource::AgentGenerated,
                    generated_at: Instant::now(),
                    confidence: 0.85,
                }
            }
            ContentType::Literal => {
                // Extract literal text after "type " or similar markers
                let literal = Self::extract_literal_text(user_intent);
                
                GeneratedContent {
                    content: literal,
                    content_type: ContentType::Literal,
                    generation_source: ContentSource::UserProvided,
                    generated_at: Instant::now(),
                    confidence: 1.0,
                }
            }
        }
    }
    
    /// Pattern-based content generation for common intents.
    /// Scaffolding: In production, would call LLM
    fn pattern_based_generation(user_intent: &str) -> String {
        let lower = user_intent.to_lowercase();
        
        // Fibonacci program generation
        if lower.contains("fibonacci") {
            return r#"def fibonacci(n):
    """Generate Fibonacci sequence up to n terms."""
    if n <= 0:
        return []
    elif n == 1:
        return [0]
    elif n == 2:
        return [0, 1]
    
    fib_sequence = [0, 1]
    for i in range(2, n):
        fib_sequence.append(fib_sequence[i-1] + fib_sequence[i-2])
    return fib_sequence

# Example usage
if __name__ == "__main__":
    n = 10
    print(f"Fibonacci sequence ({n} terms): {fibonacci(n)}")"#.to_string();
        }
        
        // Factorial program generation
        if lower.contains("factorial") {
            return r#"def factorial(n):
    """Calculate factorial of n."""
    if n < 0:
        raise ValueError("Factorial not defined for negative numbers")
    if n == 0 or n == 1:
        return 1
    return n * factorial(n - 1)

# Example usage
if __name__ == "__main__":
    for i in range(6):
        print(f"{i}! = {factorial(i)}")"#.to_string();
        }
        
        // Hello World (basic case)
        if lower.contains("hello world") {
            return "Hello World".to_string();
        }
        
        // Default: return intent as-is (fallback)
        user_intent.to_string()
    }
    
    /// Extract literal text from user intent.
    ///
    /// Only matches "type "/"enter "/"input " when they appear as command verbs:
    ///   - At the start of the string: "type Hello World"
    ///   - After "and ": "open gedit and type Hello World"
    ///
    /// Does NOT match mid-word or incidental occurrences (e.g., "text editor"
    /// contains "t" but not "type ").
    fn extract_literal_text(user_intent: &str) -> String {
        let lower = user_intent.to_lowercase();
        
        // Markers and their lengths
        let markers: &[(&str, usize)] = &[
            ("type ", 5),
            ("enter ", 6),
            ("input ", 6),
        ];
        
        for &(marker, len) in markers {
            // Check at start of string
            if lower.starts_with(marker) {
                let result = user_intent[len..].trim().to_string();
                tracing::debug!(
                    "extract_literal_text: found '{}' at start → '{}'",
                    marker.trim(), result
                );
                return result;
            }
            
            // Check after "and " (compound sentence)
            let compound = format!("and {}", marker);
            if let Some(pos) = lower.find(&compound) {
                let result = user_intent[pos + 4 + len..].trim().to_string();
                tracing::debug!(
                    "extract_literal_text: found 'and {}' at pos {} → '{}'",
                    marker.trim(), pos, result
                );
                return result;
            }
        }
        
        // Default: return full intent
        tracing::debug!(
            "extract_literal_text: no marker found, returning full intent: '{}'",
            user_intent
        );
        user_intent.to_string()
    }
    
    /// Wrap content in EvidenceWrapper for trust boundary.
    /// Per RFC 008: "Generated content marked as Agent-Generated"
    pub fn wrap_with_evidence(content: &GeneratedContent) -> crate::agent::visual_reasoning::EvidenceWrapper {
        match content.content_type {
            ContentType::Generated => {
                crate::agent::visual_reasoning::EvidenceWrapper {
                    raw_text: content.content.clone(),
                    was_truncated: false,
                    ocr_confidence: content.confidence,
                    source: crate::agent::visual_reasoning::EvidenceSource::VisualFeature, // Agent-generated
                    captured_at: content.generated_at,
                }
            }
            ContentType::Literal => {
                crate::agent::visual_reasoning::EvidenceWrapper {
                    raw_text: content.content.clone(),
                    was_truncated: false,
                    ocr_confidence: 1.0,
                    source: crate::agent::visual_reasoning::EvidenceSource::Library, // User-provided
                    captured_at: content.generated_at,
                }
            }
        }
    }
}

// ============================================================================
// Section 7: Visual Reasoner (Main Interface)
// ============================================================================

/// RFC 008: Main visual reasoning engine.
/// Per RFC 008 Section 4: "Bounded visual reasoning for novel UI elements"
pub struct VisualReasoner {
    /// Contradiction detector
    #[allow(dead_code)]
    contradiction_detector: ContradictionDetector,
    /// Semantic icon library
    icon_library: SemanticIconLibrary,
    /// Safe explorer for unknown elements
    explorer: SafeExplorer,
    /// Confidence chain for this reasoning session
    #[allow(dead_code)]
    confidence_chain: ConfidenceChain,
}

/// Output from visual reasoning.
#[derive(Debug, Clone)]
pub enum VisualReasoningOutput {
    /// Element classified with confidence
    ElementClassification(String, f32),
    /// Contradiction detected - escalate to HITL
    ContradictionDetected(VisualOcrContradiction),
    /// Insufficient confidence - requires exploration
    InsufficientConfidence,
}

impl VisualReasoner {
    /// Create new visual reasoner.
    pub fn new() -> Self {
        Self {
            contradiction_detector: ContradictionDetector,
            icon_library: SemanticIconLibrary::new(),
            explorer: SafeExplorer::new(),
            confidence_chain: ConfidenceChain::default(),
        }
    }
    
    /// Reason about element visual appearance.
    /// Per RFC 008: "Visual reasoning scope restricted: no full scene cognition"
    pub fn reason_about_element(
        &mut self,
        element: &OmniElement,
        ocr_evidence: &EvidenceWrapper,
        app_context: &AppContext,
    ) -> VisualReasoningOutput {
        // Set exploration tier based on context
        self.explorer.set_tier_for_context(app_context);
        
        // Step 1: Check for contradictions
        let visual_sig = VisualSignature {
            feature_hash: element.visual_hash.clone(),
            element_type: element.element_type.clone(),
            bbox: element.bbox,
        };
        
        if let Some(icon_semantic) = self.icon_library.lookup_semantic(&visual_sig) {
            // Check contradiction
            if let Some(contradiction) = ContradictionDetector::detect(
                element,
                &icon_semantic.semantic_meaning,
                ocr_evidence,
                &self.icon_library,
            ) {
                // Contradiction forces HITL escalation
                return VisualReasoningOutput::ContradictionDetected(contradiction);
            }
            
            // Known element: calculate confidence
            let base_confidence = icon_semantic.min_confidence;
            
            // Apply novel element ceiling if not in library
            let is_known = self.icon_library.contains(&visual_sig);
            let final_confidence = apply_novel_element_ceiling(base_confidence, is_known);
            
            return VisualReasoningOutput::ElementClassification(
                icon_semantic.semantic_meaning.clone(),
                final_confidence,
            );
        }
        
        // Novel element: insufficient confidence, requires exploration
        VisualReasoningOutput::InsufficientConfidence
    }
    
    /// Get reference to safe explorer.
    pub fn explorer(&mut self) -> &mut SafeExplorer {
        &mut self.explorer
    }
    
    /// Get reference to icon library.
    pub fn icon_library(&self) -> &SemanticIconLibrary {
        &self.icon_library
    }
}

impl Default for VisualReasoner {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_wrapper_truncate() {
        // Short text - no truncation
        let short = "Save file";
        let (result, was_truncated) = EvidenceWrapper::sentence_aware_truncate(short, 100);
        assert_eq!(result, short);
        assert!(!was_truncated);
        
        // Long text - truncation
        let long = "This is a very long sentence that should be truncated at some point. And this is another sentence.";
        let (result, was_truncated) = EvidenceWrapper::sentence_aware_truncate(long, 50);
        assert!(was_truncated);
        assert!(result.len() <= 50);
    }
    
    #[test]
    fn test_evidence_wrapper_preserve_negation() {
        // Negation context should be preserved
        let negation = "Do NOT delete this file under any circumstances. It is very important.";
        let (result, _) = EvidenceWrapper::sentence_aware_truncate(negation, 40);
        
        // Should include "Do NOT delete" not just "Do NOT"
        assert!(result.contains("delete") || result.len() >= 40);
    }
    
    #[test]
    fn test_destructive_keywords_detection() {
        let save = EvidenceWrapper::from_ocr("Save file", 0.95);
        assert!(!save.contains_destructive_keywords());
        
        let delete = EvidenceWrapper::from_ocr("Delete permanently", 0.95);
        assert!(delete.contains_destructive_keywords());
        
        let remove = EvidenceWrapper::from_ocr("Remove item", 0.95);
        assert!(remove.contains_destructive_keywords());
    }
    
    #[test]
    fn test_confidence_chain_multiplicative() {
        let chain = ConfidenceChain {
            prerequisite_confidence: 0.9,
            visual_reasoning_confidence: 0.9,
            exploration_confidence: 0.9,
        };
        
        // 0.9 * 0.9 * 0.9 = 0.729
        let final_conf = chain.calculate_final();
        assert!((final_conf - 0.729).abs() < 0.001);
    }
    
    #[test]
    fn test_novel_element_ceiling() {
        // Novel element should be capped at 0.90
        let novel = apply_novel_element_ceiling(0.95, false);
        assert_eq!(novel, 0.90);
        
        let novel_low = apply_novel_element_ceiling(0.85, false);
        assert_eq!(novel_low, 0.85); // Below ceiling, unchanged
        
        // Known element should not be capped
        let known = apply_novel_element_ceiling(0.95, true);
        assert_eq!(known, 0.95);
    }
    
    #[test]
    fn test_icon_library_default_icons() {
        let library = SemanticIconLibrary::new();
        
        // Trash icon should be destructive
        let trash_sig = VisualSignature {
            feature_hash: "trash_icon".to_string(),
            element_type: "icon".to_string(),
            bbox: [0, 0, 32, 32],
        };
        let trash = library.lookup_semantic(&trash_sig).unwrap();
        assert!(trash.is_destructive);
        assert_eq!(trash.action_verb, "delete");
        
        // Plus icon should not be destructive
        let plus_sig = VisualSignature {
            feature_hash: "plus_sign".to_string(),
            element_type: "button".to_string(),
            bbox: [0, 0, 32, 32],
        };
        let plus = library.lookup_semantic(&plus_sig).unwrap();
        assert!(!plus.is_destructive);
        assert_eq!(plus.action_verb, "create");
    }
    
    #[test]
    fn test_exploration_tier_payment_forbidden() {
        let mut explorer = SafeExplorer::new();
        
        // Payment page should be Forbidden
        let payment_context = AppContext {
            app_name: "Chrome".to_string(),
            window_title: "Checkout".to_string(),
            current_url: Some("https://example.com/checkout".to_string()),
            is_payment_page: true,
        };
        
        explorer.set_tier_for_context(&payment_context);
        assert_eq!(explorer.current_tier(), ExplorationTier::Forbidden);
        assert!(!explorer.can_explore());
        assert!(!explorer.can_hover());
    }
    
    #[test]
    fn test_exploration_tier_safe_apps() {
        let mut explorer = SafeExplorer::new();
        
        // Gedit should be Safe
        let gedit_context = AppContext {
            app_name: "gedit".to_string(),
            window_title: "Untitled Document".to_string(),
            current_url: None,
            is_payment_page: false,
        };
        
        explorer.set_tier_for_context(&gedit_context);
        assert_eq!(explorer.current_tier(), ExplorationTier::Safe);
        assert!(explorer.can_explore());
        assert!(explorer.can_click());
    }
    
    #[test]
    fn test_contradiction_trash_vs_save() {
        let library = SemanticIconLibrary::new();
        
        // Create element with trash icon
        let trash_element = OmniElement {
            id: "btn_1".to_string(),
            element_type: "icon".to_string(),
            label: "Save".to_string(), // OCR says "Save"
            label_wrapped: "<evidence>Save</evidence>".to_string(),
            bbox: [100, 100, 132, 132],
            confidence: 0.95,
            monitor_id: 0,
            dpi_scale: 1.0,
            visual_hash: "trash_icon".to_string(), // Visual is trash
        };
        
        let ocr_evidence = EvidenceWrapper::from_ocr("Save", 0.90);
        
        // This should detect contradiction: trash icon + "Save" text
        let contradiction = ContradictionDetector::detect(
            &trash_element,
            "delete or remove item",
            &ocr_evidence,
            &library,
        );
        
        assert!(contradiction.is_some());
        let c = contradiction.unwrap();
        assert_eq!(c.contradiction_type, ContradictionType::DestructiveIconBenignText);
        assert!(c.forces_zero_confidence);
    }
    
    #[test]
    fn test_contradiction_no_conflict() {
        let library = SemanticIconLibrary::new();
        
        // Create element with save icon
        let save_element = OmniElement {
            id: "btn_2".to_string(),
            element_type: "icon".to_string(),
            label: "Save".to_string(),
            label_wrapped: "<evidence>Save</evidence>".to_string(),
            bbox: [100, 100, 132, 132],
            confidence: 0.95,
            monitor_id: 0,
            dpi_scale: 1.0,
            visual_hash: "floppy_disk".to_string(), // Visual is save
        };
        
        let ocr_evidence = EvidenceWrapper::from_ocr("Save", 0.90);
        
        // This should NOT detect contradiction: save icon + "Save" text
        let contradiction = ContradictionDetector::detect(
            &save_element,
            "save changes",
            &ocr_evidence,
            &library,
        );
        
        assert!(contradiction.is_none());
    }
    
    #[test]
    fn test_visual_reasoner_contradiction_escalation() {
        let mut reasoner = VisualReasoner::new();
        
        let trash_element = OmniElement {
            id: "btn_1".to_string(),
            element_type: "icon".to_string(),
            label: "Save".to_string(),
            label_wrapped: "<evidence>Save</evidence>".to_string(),
            bbox: [100, 100, 132, 132],
            confidence: 0.95,
            monitor_id: 0,
            dpi_scale: 1.0,
            visual_hash: "trash_icon".to_string(),
        };
        
        let ocr_evidence = EvidenceWrapper::from_ocr("Save", 0.90);
        let app_context = AppContext {
            app_name: "TestApp".to_string(),
            window_title: "Test".to_string(),
            current_url: None,
            is_payment_page: false,
        };
        
        let result = reasoner.reason_about_element(&trash_element, &ocr_evidence, &app_context);
        
        // Should detect contradiction and escalate
        match result {
            VisualReasoningOutput::ContradictionDetected(c) => {
                assert!(c.forces_zero_confidence);
                assert_eq!(c.visual_evidence, "delete or remove item");
                assert_eq!(c.ocr_evidence, "Save");
            }
            _ => panic!("Expected contradiction detection, got {:?}", result),
        }
    }
}
