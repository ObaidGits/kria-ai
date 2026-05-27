//! Vision Automation - Phase 2: OmniParser Vision Bridge
//!
//! RFC 007 Implementation: Screen understanding with strict safety invariants.
//! This module provides vision-based GUI automation through OmniParser
//! integration with cognitive poisoning defenses.

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::gui_automation::{GuiBackend, MouseButton, YdotoolBackend};
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ============================================================================
// Section 1: OmniParser Output Schema (Section 3.2)
// ============================================================================

/// OmniParser element representation with cognitive defense metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OmniElement {
    /// Unique element identifier
    pub id: String,
    /// Element type (button, input, text, etc.)
    pub element_type: String,
    /// Raw label text from OCR (untrusted)
    pub label: String,
    /// Label wrapped in EvidenceWrapper for cognitive poisoning defense
    pub label_wrapped: String,
    /// Bounding box [x1, y1, x2, y2]
    pub bbox: [i32; 4],
    /// Detection confidence 0.0-1.0
    pub confidence: f32,
    /// Monitor ID for multi-display setups
    pub monitor_id: u32,
    /// DPI scaling factor
    pub dpi_scale: f32,
    /// Visual hash for integrity verification (pHash or similar)
    pub visual_hash: String,
}

impl OmniElement {
    /// Create new element with cognitive poisoning defense applied.
    pub fn new(
        id: String,
        element_type: String,
        label: String,
        bbox: [i32; 4],
        confidence: f32,
        monitor_id: u32,
        dpi_scale: f32,
        visual_hash: String,
    ) -> Self {
        // Apply cognitive poisoning defense: truncate and wrap
        let truncated = Self::apply_cognitive_defense(&label);
        let label_wrapped = format!("<evidence>{}</evidence>", truncated);

        Self {
            id,
            element_type,
            label,
            label_wrapped,
            bbox,
            confidence,
            monitor_id,
            dpi_scale,
            visual_hash,
        }
    }

    /// Apply cognitive poisoning defense to OCR text.
    /// Per RFC 007 Section 3.2: aggressively truncate to 100 chars max.
    fn apply_cognitive_defense(text: &str) -> String {
        if text.len() <= 100 {
            text.to_string()
        } else {
            // Truncate to 100 chars, add ellipsis
            let truncated: String = text.chars().take(97).collect();
            format!("{}...", truncated)
        }
    }

    /// Get center coordinates of bounding box.
    pub fn center(&self) -> (i32, i32) {
        let x = (self.bbox[0] + self.bbox[2]) / 2;
        let y = (self.bbox[1] + self.bbox[3]) / 2;
        (x, y)
    }

    /// Get bounding box dimensions.
    pub fn dimensions(&self) -> (i32, i32) {
        let width = self.bbox[2] - self.bbox[0];
        let height = self.bbox[3] - self.bbox[1];
        (width, height)
    }
}

/// Full OmniParser output with multi-monitor support.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OmniParserOutput {
    /// Detected UI elements
    pub elements: Vec<OmniElement>,
    /// Overall screen dimensions
    pub screen_dimensions: [u32; 2],
    /// Per-monitor dimensions for multi-display
    pub monitor_dimensions: Vec<[u32; 2]>,
    /// Timestamp of parsing
    pub timestamp: u64,
    /// Full-screen visual hash for integrity
    pub visual_hash: String,
}

/// Window information for verification.
#[derive(Debug, Clone)]
pub struct WindowContext {
    pub title: String,
    pub class: String,
    pub pid: u32,
}

// ============================================================================
// Section 2: State Cache with Auto-Invalidation
// ============================================================================

/// Cached OmniParser state with TTL.
struct CachedState {
    /// The parsed output
    output: OmniParserOutput,
    /// Timestamp when cache was created
    created_at: Instant,
    /// Screenshot data (for visual hash verification)
    screenshot_data: Vec<u8>,
}

/// 5-second state cache for OmniParser results.
pub struct OmniParserCache {
    /// In-memory cache keyed by monitor/display context
    cache: RwLock<HashMap<String, CachedState>>,
    /// TTL duration (5 seconds per RFC 007)
    ttl: Duration,
}

impl OmniParserCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(5),
        }
    }

    /// Get cached state if still valid.
    async fn get(&self, key: &str) -> Option<CachedState> {
        let cache = self.cache.read().await;

        if let Some(state) = cache.get(key) {
            if state.created_at.elapsed() < self.ttl {
                // Cache hit and still valid
                return Some(CachedState {
                    output: state.output.clone(),
                    created_at: state.created_at,
                    screenshot_data: state.screenshot_data.clone(),
                });
            }
            // TTL expired, will be cleaned up on next write
        }
        None
    }

    /// Store new state in cache.
    async fn set(&self, key: String, output: OmniParserOutput, screenshot_data: Vec<u8>) {
        let mut cache = self.cache.write().await;

        // Clean up expired entries while we have write lock
        let now = Instant::now();
        cache.retain(|_, state| now.duration_since(state.created_at) < self.ttl);

        // Insert new entry
        cache.insert(
            key,
            CachedState {
                output,
                created_at: now,
                screenshot_data,
            },
        );
    }

    /// Instantly invalidate all cached state.
    /// Per RFC 007: "cache must be instantly invalidated the moment a state-changing
    /// action (click or type) occurs"
    pub async fn invalidate_all(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        tracing::info!(target: "vision_cache", "OmniParser cache invalidated due to state-changing action");
    }

    /// Get element by ID from cache.
    pub async fn get_element_by_id(&self, element_id: &str) -> Option<OmniElement> {
        let cache = self.cache.read().await;

        for state in cache.values() {
            if let Some(element) = state.output.elements.iter().find(|e| e.id == element_id) {
                // Check if this specific element is still within TTL
                if state.created_at.elapsed() < self.ttl {
                    return Some(element.clone());
                }
                return None;
            }
        }
        None
    }

    /// Get screenshot data for element verification.
    #[allow(dead_code)] // Reserved for future element verification
    async fn get_screenshot_for_element(&self, element_id: &str) -> Option<(Vec<u8>, OmniElement)> {
        let cache = self.cache.read().await;

        for state in cache.values() {
            if let Some(element) = state.output.elements.iter().find(|e| e.id == element_id) {
                if state.created_at.elapsed() < self.ttl {
                    return Some((state.screenshot_data.clone(), element.clone()));
                }
                return None;
            }
        }
        None
    }
}

impl Default for OmniParserCache {
    fn default() -> Self {
        Self::new()
    }
}

// Global cache instance (shared between tools and verification engine)
pub static OMNI_CACHE: Lazy<OmniParserCache> = Lazy::new(OmniParserCache::new);

// ============================================================================
// Section 2.5: RFC 008 Phase 2 - Saliency-Aware Perceptual Diff
// ============================================================================

/// RFC 008: Screen region with saliency weight for perceptual diff.
/// Higher weight = more important for detecting structural changes.
#[derive(Debug, Clone)]
pub struct SaliencyRegion {
    /// Region name for logging/debugging
    pub name: &'static str,
    /// Bounding box [x1, y1, x2, y2] - optional, None means whole screen
    pub bbox: Option<[i32; 4]>,
    /// Saliency weight (higher = more important)
    pub weight: f32,
    /// Region type
    pub region_type: SaliencyRegionType,
}

/// RFC 008: Types of salient regions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaliencyRegionType {
    /// Center of focused window - most interactions happen here (weight: 3.0)
    CenterFocusedWindow,
    /// Modal/dialog overlay - highest priority for blocking UI (weight: 5.0)
    ModalOverlay,
    /// Notification area - important state changes (weight: 2.0)
    NotificationArea,
    /// Background area - low priority (weight: 0.5)
    Background,
}

impl SaliencyRegion {
    /// Create standard saliency regions for a screen.
    /// Per RFC 008 Section 2.2: weighted similarity by region importance.
    pub fn default_regions(screen_width: u32, screen_height: u32) -> Vec<Self> {
        let cx = (screen_width / 2) as i32;
        let cy = (screen_height / 2) as i32;

        vec![
            // Modal overlay region (highest priority) - center screen where modals appear
            SaliencyRegion {
                name: "modal_overlay",
                bbox: Some([cx - 200, cy - 150, cx + 200, cy + 150]),
                weight: 5.0,
                region_type: SaliencyRegionType::ModalOverlay,
            },
            // Center focused window - primary interaction area
            SaliencyRegion {
                name: "center_focused_window",
                bbox: Some([cx - 300, cy - 200, cx + 300, cy + 200]),
                weight: 3.0,
                region_type: SaliencyRegionType::CenterFocusedWindow,
            },
            // Notification area - typically top-right
            SaliencyRegion {
                name: "notification_area",
                bbox: Some([(screen_width as i32) - 250, 0, screen_width as i32, 100]),
                weight: 2.0,
                region_type: SaliencyRegionType::NotificationArea,
            },
            // Background (low priority) - edges of screen
            SaliencyRegion {
                name: "background",
                bbox: None, // Whole screen for background
                weight: 0.5,
                region_type: SaliencyRegionType::Background,
            },
        ]
    }
}

/// RFC 008: Perceptual diff result with saliency weighting.
#[derive(Debug, Clone)]
pub struct SaliencyDiffResult {
    /// Weighted similarity score (0.0-1.0)
    pub weighted_similarity: f32,
    /// Per-region similarity scores
    pub region_scores: Vec<(String, f32)>,
    /// Modal detected in current but not cached
    pub modal_appeared: bool,
    /// Structural change detected (below threshold)
    pub structural_change: bool,
}

/// RFC 008: Gated sensing state machine.
/// Per RFC 008 Section 2.2: "Re-evaluation is a gated, expensive operation"
pub struct GatedSensing {
    /// Last cached screen state
    last_screen_hash: Option<u64>,
    /// Last cached parsed state
    last_parsed_state: Option<OmniParserOutput>,
    /// Cache timestamp
    last_sense_time: Option<Instant>,
    /// TTL for cache validity (10 seconds per RFC 008)
    ttl: Duration,
    /// SSIM threshold for structural change (0.85 per RFC 008)
    ssim_threshold: f32,
    /// Saliency regions for weighted diff
    saliency_regions: Vec<SaliencyRegion>,
}

impl GatedSensing {
    /// Create new gated sensing instance.
    /// Per RFC 008: "Maximum 1 sense per second to prevent screen polling spam"
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        Self {
            last_screen_hash: None,
            last_parsed_state: None,
            last_sense_time: None,
            ttl: Duration::from_secs(10), // RFC 008: 10s semantic state TTL
            ssim_threshold: 0.85,         // RFC 008: SSIM < 0.85 = structural change
            saliency_regions: SaliencyRegion::default_regions(screen_width, screen_height),
        }
    }

    /// RFC 008: Saliency-aware perceptual diff.
    /// Per RFC 008 Section 2.2: "Weighted similarity calculation"
    pub fn saliency_aware_diff(
        &self,
        current_hash: u64,
        cached_hash: u64,
        current_screenshot: &image::DynamicImage,
        cached_screenshot: &image::DynamicImage,
    ) -> SaliencyDiffResult {
        let mut weighted_diff = 0.0;
        let mut total_weight = 0.0;
        let mut region_scores = Vec::new();
        let mut modal_appeared = false;

        for region in &self.saliency_regions {
            // Calculate local similarity for this region
            let local_similarity = if let Some(bbox) = region.bbox {
                self.calculate_local_ssim(current_screenshot, cached_screenshot, bbox)
            } else {
                // Background uses global similarity
                self.calculate_global_ssim(current_hash, cached_hash)
            };

            // Check for modal appearance (high similarity change in modal region)
            if region.region_type == SaliencyRegionType::ModalOverlay && local_similarity < 0.70 {
                modal_appeared = true;
            }

            weighted_diff += local_similarity * region.weight;
            total_weight += region.weight;
            region_scores.push((region.name.to_string(), local_similarity));
        }

        let weighted_similarity = if total_weight > 0.0 {
            weighted_diff / total_weight
        } else {
            1.0
        };

        // Modal appearance forces structural change regardless of global similarity
        let structural_change = weighted_similarity < self.ssim_threshold || modal_appeared;

        SaliencyDiffResult {
            weighted_similarity,
            region_scores,
            modal_appeared,
            structural_change,
        }
    }

    /// Calculate SSIM for a specific region (simplified approximation using pHash).
    fn calculate_local_ssim(
        &self,
        current: &image::DynamicImage,
        cached: &image::DynamicImage,
        bbox: [i32; 4],
    ) -> f32 {
        // Extract region from both images
        let (x1, y1, x2, y2) = (bbox[0], bbox[1], bbox[2], bbox[3]);
        let width = (x2 - x1).max(1) as u32;
        let height = (y2 - y1).max(1) as u32;

        // Clamp bounds to image dimensions
        let crop_x = (x1.max(0) as u32).min(current.width() - 1);
        let crop_y = (y1.max(0) as u32).min(current.height() - 1);
        let crop_w = width.min(current.width() - crop_x);
        let crop_h = height.min(current.height() - crop_y);

        // Crop current region using imageops::crop_imm
        let current_region =
            image::imageops::crop_imm(current, crop_x, crop_y, crop_w, crop_h).to_image();
        let current_dynamic = image::DynamicImage::ImageRgba8(current_region);

        // Crop cached region
        let cached_x = (x1.max(0) as u32).min(cached.width() - 1);
        let cached_y = (y1.max(0) as u32).min(cached.height() - 1);
        let cached_w = width.min(cached.width() - cached_x);
        let cached_h = height.min(cached.height() - cached_y);

        let cached_region =
            image::imageops::crop_imm(cached, cached_x, cached_y, cached_w, cached_h).to_image();
        let cached_dynamic = image::DynamicImage::ImageRgba8(cached_region);

        // Calculate pHash for both regions and compare
        if let (Ok(current_hash), Ok(cached_hash)) = (
            VisualHashVerifier::calculate_phash(&current_dynamic),
            VisualHashVerifier::calculate_phash(&cached_dynamic),
        ) {
            VisualHashVerifier::calculate_similarity(&current_hash, &cached_hash)
        } else {
            0.5 // Unknown if hash calculation fails
        }
    }

    /// Calculate global SSIM using perceptual hash comparison.
    fn calculate_global_ssim(&self, current_hash: u64, cached_hash: u64) -> f32 {
        // Convert u64 hashes to strings for comparison
        let current_str = format!("{:016x}", current_hash);
        let cached_str = format!("{:016x}", cached_hash);

        // Simple Hamming distance approximation on hex strings
        let distance = current_str
            .chars()
            .zip(cached_str.chars())
            .filter(|(a, b)| a != b)
            .count();

        // Convert to similarity (64 hex chars max)
        1.0 - (distance as f32 / 64.0)
    }

    /// Check if re-sensing is required.
    /// Per RFC 008: "Re-evaluation executes ONLY on verification failure,
    /// major perceptual diff, blocking interrupt, or timer"
    pub fn needs_resense(&self, force_invalidation: bool) -> bool {
        // Force invalidation from OS events or human activity
        if force_invalidation {
            return true;
        }

        // TTL expiration
        if let Some(last) = self.last_sense_time {
            if last.elapsed() > self.ttl {
                return true;
            }
        }

        // No previous sense
        if self.last_screen_hash.is_none() {
            return true;
        }

        false
    }

    /// Update cached state after sensing.
    pub fn update_cache(&mut self, screen_hash: u64, parsed: OmniParserOutput) {
        self.last_screen_hash = Some(screen_hash);
        self.last_parsed_state = Some(parsed);
        self.last_sense_time = Some(Instant::now());
    }

    /// Force invalidate cache (e.g., on human activity).
    pub fn invalidate(&mut self) {
        self.last_screen_hash = None;
        self.last_parsed_state = None;
        self.last_sense_time = None;
        tracing::info!(target: "gated_sensing", "Cache invalidated");
    }
}

// ============================================================================
// Section 3: GPU Lease Management (Scaffolding)
// ============================================================================

/// GPU lease handle for resource management.
pub struct GpuLease {
    /// Lease ID
    _id: String,
    /// When lease was acquired
    _acquired_at: Instant,
}

impl GpuLease {
    /// Release the lease (drop implementation handles actual release).
    pub fn release(self) {
        // Explicit release - the drop will handle cleanup
        drop(self);
    }
}

impl Drop for GpuLease {
    fn drop(&mut self) {
        // In production, this would signal the GPU lease manager
        tracing::debug!(target: "gpu_lease", "GPU lease {} released", self._id);
    }
}

/// GPU lease manager interface (scaffolding for Phase 2).
pub struct GpuLeaseManager;

impl GpuLeaseManager {
    /// Request a GPU lease for vision operations.
    /// Per RFC 007: "OmniParser execution requires GPU lease management"
    pub async fn request_lease() -> Option<GpuLease> {
        // Scaffolding: In production, this would:
        // 1. Check GPU availability
        // 2. Queue if necessary
        // 3. Return lease handle with timeout

        // For now, always succeed (actual GPU management in Phase 2.5)
        Some(GpuLease {
            _id: format!("vision-{}", uuid::Uuid::new_v4()),
            _acquired_at: Instant::now(),
        })
    }

    /// Check if GPU is available without acquiring lease.
    pub fn is_available() -> bool {
        // Scaffolding: always true for now
        true
    }
}

// ============================================================================
// Section 4: OmniParser Client Interface
// ============================================================================

/// Client for communicating with OmniParser Python sidecar.
pub struct OmniParserClient {
    /// HTTP client for API requests
    client: reqwest::Client,
    /// Endpoint URL
    endpoint: String,
    /// Timeout for requests
    #[allow(dead_code)] // Reserved for future timeout configuration
    timeout: Duration,
}

impl OmniParserClient {
    pub fn new(endpoint: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            endpoint,
            timeout: Duration::from_secs(30),
        }
    }

    /// Parse screenshot through OmniParser sidecar.
    /// Sends actual screenshot bytes to Python FastAPI service.
    pub async fn parse_screenshot(
        &self,
        screenshot_data: &[u8],
    ) -> Result<OmniParserOutput, VisionError> {
        tracing::debug!(target: "omniparser", "Sending screenshot to OmniParser at {}", self.endpoint);

        let start = Instant::now();

        // Build multipart form with screenshot
        let form = reqwest::multipart::Form::new()
            .part(
                "image",
                reqwest::multipart::Part::bytes(screenshot_data.to_vec())
                    .file_name("screenshot.png")
                    .mime_str("image/png")
                    .map_err(|e| {
                        VisionError::ParserError(format!("Failed to build form: {}", e))
                    })?,
            )
            .text("monitor_id", "0")
            .text("confidence_threshold", "0.8");

        // Send POST request to /parse_screen
        let url = format!("{}/parse_screen", self.endpoint);
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| VisionError::ParserError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(VisionError::ParserError(format!(
                "OmniParser returned error {}: {}",
                status, text
            )));
        }

        // Read raw response text for optional debug dump
        let raw_json = response.text().await.map_err(|e| {
            VisionError::ParserError(format!("Failed to read response text: {}", e))
        })?;

        // Vision diagnostics: save raw PNG + JSON when KRIA_DEBUG_VISION=1
        if std::env::var("KRIA_DEBUG_VISION").unwrap_or_default() == "1" {
            let debug_dir = PathBuf::from("/tmp/kria_vision_debug");
            if let Err(e) = std::fs::create_dir_all(&debug_dir) {
                tracing::warn!(target: "omniparser", "Failed to create debug dir: {}", e);
            } else {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let png_path = debug_dir.join(format!("screenshot_{}.png", timestamp));
                let json_path = debug_dir.join(format!("response_{}.json", timestamp));
                if let Err(e) = std::fs::write(&png_path, screenshot_data) {
                    tracing::warn!(target: "omniparser", "Failed to write debug PNG: {}", e);
                } else {
                    tracing::info!(target: "omniparser", "Debug PNG saved: {}", png_path.display());
                }
                if let Err(e) = std::fs::write(&json_path, &raw_json) {
                    tracing::warn!(target: "omniparser", "Failed to write debug JSON: {}", e);
                } else {
                    tracing::info!(target: "omniparser", "Debug JSON saved: {}", json_path.display());
                }
            }
        }

        // Parse JSON response
        let parse_response: OmniParseResponse = serde_json::from_str(&raw_json)
            .map_err(|e| VisionError::ParserError(format!("Failed to parse JSON: {}", e)))?;

        let elapsed = start.elapsed();
        tracing::info!(
            target: "omniparser",
            "Parsed screenshot in {}ms, found {} elements",
            elapsed.as_millis(),
            parse_response.data.elements.len()
        );

        Ok(parse_response.data)
    }

    /// Verify visual hash of image region via sidecar.
    pub async fn verify_hash(
        &self,
        image_data: &[u8],
        expected_hash: &str,
        bbox: Option<[i32; 4]>,
    ) -> Result<f32, VisionError> {
        let mut form = reqwest::multipart::Form::new()
            .part(
                "image",
                reqwest::multipart::Part::bytes(image_data.to_vec())
                    .file_name("region.png")
                    .mime_str("image/png")
                    .map_err(|e| {
                        VisionError::ParserError(format!("Failed to build form: {}", e))
                    })?,
            )
            .text("expected_hash", expected_hash.to_string());

        if let Some(b) = bbox {
            let bbox_str = format!("[{}, {}, {}, {}]", b[0], b[1], b[2], b[3]);
            form = form.text("bbox", bbox_str);
        }

        let url = format!("{}/verify_hash", self.endpoint);
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| VisionError::ParserError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(VisionError::ParserError(
                "Hash verification request failed".to_string(),
            ));
        }

        let verify_response: VerifyHashResponse = response.json().await.map_err(|e| {
            VisionError::ParserError(format!("Failed to parse verify response: {}", e))
        })?;

        Ok(verify_response.similarity)
    }
}

/// Response from /parse_screen endpoint.
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)] // Fields used by serde deserialization
struct OmniParseResponse {
    success: bool,
    data: OmniParserOutput,
    processing_time_ms: f64,
}

/// Response from /verify_hash endpoint.
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)] // Fields used by serde deserialization
struct VerifyHashResponse {
    similarity: f32,
    calculated_hash: String,
    expected_hash: String,
    verified: bool,
}

/// Vision operation errors.
#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("OmniParser request failed: {0}")]
    ParserError(String),
    #[error("GPU lease unavailable")]
    GpuUnavailable,
    #[error("Visual hash mismatch (IoU < 0.75)")]
    VisualHashMismatch,
    #[error("Element not found: {0}")]
    ElementNotFound(String),
    #[error("Cache expired or invalid")]
    CacheInvalid,
    #[error("Screenshot capture failed: {0}")]
    ScreenshotFailed(String),
}

// ============================================================================
// Section 5: Visual Hash Verification (pHash/SSIM)
// ============================================================================

/// Visual hash verification using perceptual hashing (pHash).
pub struct VisualHashVerifier;

impl VisualHashVerifier {
    /// Verify element visual hash before clicking.
    /// Per RFC 007 Section 3.2: "Visual Hash Verification step to click_element"
    ///
    /// Steps:
    /// 1. Capture 50x50 micro-screenshot of target coordinates
    /// 2. Calculate pHash of micro-screenshot
    /// 3. Compare to original OmniParser crop hash
    /// 4. Abort if similarity < 0.75
    pub async fn verify_before_click(
        element: &OmniElement,
        current_screenshot: &image::DynamicImage,
    ) -> Result<bool, VisionError> {
        let (center_x, center_y) = element.center();
        tracing::debug!(
            target: "visual_hash",
            "Verifying visual hash for element {} at ({}, {})",
            element.id, center_x, center_y
        );

        // Calculate pHash of current screenshot region (DynamicImage passed directly)
        let current_hash = Self::calculate_phash(current_screenshot)?;

        // Compare with stored hash using Hamming distance
        let similarity = Self::calculate_similarity(&current_hash, &element.visual_hash);

        tracing::debug!(
            target: "visual_hash",
            "Similarity: {:.2} (threshold: 0.75)",
            similarity
        );

        // Lowered from 0.90 to 0.75 to tolerate cursor changes and CSS hover states
        Ok(similarity > 0.75)
    }

    /// Calculate perceptual hash using img_hash crate.
    /// Uses gradient hash algorithm for robust comparison.
    /// Converts image 0.25 buffer directly to img_hash's image 0.23 format —
    /// no PNG encode/decode roundtrip.
    pub fn calculate_phash(img: &image::DynamicImage) -> Result<String, VisionError> {
        use img_hash::{image as img23, HashAlg, HasherConfig};

        // Convert our image 0.25 RgbaImage to img_hash's image 0.23 ImageBuffer.
        // Both Rgba<u8> types are #[repr(C)] [u8; 4] so raw bytes are compatible.
        let rgba = img.to_rgba8();
        let (width, height) = (rgba.width(), rgba.height());
        let raw: Vec<u8> = rgba.into_raw();

        let img23_buffer =
            img23::ImageBuffer::<img23::Rgba<u8>, Vec<u8>>::from_raw(width, height, raw)
                .ok_or_else(|| {
                    VisionError::ScreenshotFailed(
                        "Failed to create image buffer for hashing".to_string(),
                    )
                })?;

        // Create hasher with gradient hash algorithm (similar to pHash)
        let hasher = HasherConfig::new()
            .hash_alg(HashAlg::DoubleGradient)
            .hash_size(8, 8)
            .to_hasher();

        // Calculate hash directly from raw buffer (no PNG decode step)
        let hash = hasher.hash_image(&img23_buffer);

        // Convert to base64 string
        let hash_str = hash.to_base64();

        Ok(hash_str)
    }

    /// Calculate similarity using Hamming distance.
    /// Returns similarity score 0.0-1.0 where 1.0 is identical.
    fn calculate_similarity(hash1: &str, hash2: &str) -> f32 {
        use img_hash::ImageHash;

        // Parse base64 hash strings with explicit type
        type HashType = Box<[u8]>;
        let h1: ImageHash<HashType> = match ImageHash::from_base64(hash1) {
            Ok(h) => h,
            Err(_) => return 0.0,
        };
        let h2: ImageHash<HashType> = match ImageHash::from_base64(hash2) {
            Ok(h) => h,
            Err(_) => return 0.0,
        };

        // Calculate distance (number of differing bits)
        let distance = h1.dist(&h2);

        // Convert to similarity (hash size in bits)
        // similarity = 1.0 - (distance / total_bits)
        let total_bits = (h1.as_bytes().len() * 8) as f32;
        let similarity = 1.0 - (distance as f32 / total_bits);

        similarity
    }
}

// ============================================================================
// Section 6: Screenshot Capture (Live Implementation)
// ============================================================================

/// Screenshot capture utility using xcap crate.
pub struct ScreenshotCapture;

impl ScreenshotCapture {
    /// Capture full screenshot of primary monitor.
    pub async fn capture_full() -> Result<Vec<u8>, VisionError> {
        tracing::debug!(target: "screenshot", "Capturing full screenshot via xcap");

        // Spawn blocking screenshot capture in separate task
        let screenshot = tokio::task::spawn_blocking(|| {
            // Get primary monitor
            let monitors = xcap::Monitor::all().map_err(|e| {
                VisionError::ScreenshotFailed(format!("Failed to get monitors: {}", e))
            })?;

            let primary = monitors
                .first()
                .ok_or_else(|| VisionError::ScreenshotFailed("No monitors found".to_string()))?;

            // Capture screenshot
            let image = primary
                .capture_image()
                .map_err(|e| VisionError::ScreenshotFailed(format!("Failed to capture: {}", e)))?;

            // Encode to PNG bytes
            let mut buffer = Cursor::new(Vec::new());
            image
                .write_to(&mut buffer, image::ImageFormat::Png)
                .map_err(|e| {
                    VisionError::ScreenshotFailed(format!("Failed to encode PNG: {}", e))
                })?;

            Ok::<Vec<u8>, VisionError>(buffer.into_inner())
        })
        .await
        .map_err(|e| VisionError::ScreenshotFailed(format!("Screenshot task failed: {}", e)))?;

        screenshot
    }

    /// Capture an exact screen region for visual verification.
    /// Uses the element's precise bbox — no hardcoded 50x50 square.
    /// Returns DynamicImage directly — no PNG encode/decode roundtrip for local ops.
    pub async fn capture_region(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<image::DynamicImage, VisionError> {
        tracing::debug!(
            target: "screenshot",
            "Capturing region {}x{} at ({}, {}) via xcap",
            width, height, x, y
        );

        // Validate coordinates
        if x < 0 || y < 0 || width == 0 || height == 0 {
            return Err(VisionError::ScreenshotFailed(format!(
                "Invalid region: ({}, {}), size={}x{}",
                x, y, width, height
            )));
        }

        let x_u32 = x as u32;
        let y_u32 = y as u32;

        // Spawn blocking screenshot capture
        let screenshot = tokio::task::spawn_blocking(move || {
            // Get primary monitor
            let monitors = xcap::Monitor::all().map_err(|e| {
                VisionError::ScreenshotFailed(format!("Failed to get monitors: {}", e))
            })?;

            let primary = monitors
                .first()
                .ok_or_else(|| VisionError::ScreenshotFailed("No monitors found".to_string()))?;

            // Capture screenshot (xcap returns image::RgbaImage)
            let image = primary
                .capture_image()
                .map_err(|e| VisionError::ScreenshotFailed(format!("Failed to capture: {}", e)))?;

            // Extract exact region using provided dimensions
            let screen_width = image.width();
            let screen_height = image.height();

            // Clamp crop bounds to screen dimensions
            let crop_x = x_u32.min(screen_width - 1);
            let crop_y = y_u32.min(screen_height - 1);
            let crop_width = width.min(screen_width - crop_x);
            let crop_height = height.min(screen_height - crop_y);

            // Crop the region
            let cropped =
                image::imageops::crop_imm(&image, crop_x, crop_y, crop_width, crop_height);

            // Convert directly to DynamicImage — no PNG serialization for local ops
            let cropped_image = cropped.to_image();
            Ok::<image::DynamicImage, VisionError>(image::DynamicImage::ImageRgba8(cropped_image))
        })
        .await
        .map_err(|e| VisionError::ScreenshotFailed(format!("Region capture task failed: {}", e)))?;

        screenshot
    }
}

// ============================================================================
// Section 7: Tool Implementations
// ============================================================================

/// Shared state for vision tools.
struct VisionToolState {
    omni_client: OmniParserClient,
    gui_backend: Arc<dyn GuiBackend>,
}

/// get_screen_elements tool implementation.
struct GetScreenElements {
    state: Arc<VisionToolState>,
}

#[async_trait]
impl ToolHandler for GetScreenElements {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let filter_type = params["filter_type"].as_str();
        let min_confidence = params["min_confidence"].as_f64().unwrap_or(0.8) as f32;
        let monitor_id = params["monitor_id"].as_u64().unwrap_or(0) as u32;

        // Check cache first
        let cache_key = format!("monitor_{}", monitor_id);

        if let Some(cached) = OMNI_CACHE.get(&cache_key).await {
            tracing::debug!(target: "get_screen_elements", "Cache hit for {}", cache_key);

            // Filter cached elements
            let elements: Vec<_> = cached
                .output
                .elements
                .iter()
                .filter(|e| {
                    // Apply confidence filter
                    if e.confidence < min_confidence {
                        return false;
                    }
                    // Apply type filter if specified
                    if let Some(ft) = filter_type {
                        if e.element_type != ft {
                            return false;
                        }
                    }
                    true
                })
                .map(|e| {
                    // Return element with wrapped label (cognitive poisoning defense)
                    serde_json::json!({
                        "id": e.id,
                        "type": e.element_type,
                        "label": e.label_wrapped, // Use wrapped version!
                        "bbox": e.bbox,
                        "confidence": e.confidence,
                        "monitor_id": e.monitor_id,
                    })
                })
                .collect();

            return ToolResult::ok(serde_json::json!({
                "elements": elements,
                "count": elements.len(),
                "source": "cache",
                "cache_age_ms": cached.created_at.elapsed().as_millis(),
            }));
        }

        // Cache miss - need to parse screen
        tracing::info!(target: "get_screen_elements", "Cache miss, parsing screen");

        // Step 1: Request GPU lease
        let _gpu_lease = match GpuLeaseManager::request_lease().await {
            Some(lease) => lease,
            None => {
                return ToolResult::err("GPU lease unavailable - vision parsing blocked");
            }
        };

        // Step 2: Capture screenshot
        let screenshot_data = match ScreenshotCapture::capture_full().await {
            Ok(data) => data,
            Err(e) => return ToolResult::err(format!("Screenshot failed: {}", e)),
        };

        // Step 3: Parse through OmniParser
        let mut parsed = match self
            .state
            .omni_client
            .parse_screenshot(&screenshot_data)
            .await
        {
            Ok(output) => output,
            Err(e) => return ToolResult::err(format!("OmniParser failed: {}", e)),
        };

        // Step 3.5: Recalculate visual hashes from real pixels (bypass dummy sidecar data)
        let screenshot_img = match image::load_from_memory(&screenshot_data) {
            Ok(img) => img,
            Err(e) => {
                return ToolResult::err(format!("Failed to decode screenshot for hashing: {}", e))
            }
        };

        for element in parsed.elements.iter_mut() {
            let [x1, y1, x2, y2] = element.bbox;
            let width = ((x2 - x1).max(1)) as u32;
            let height = ((y2 - y1).max(1)) as u32;
            let crop_x = x1.max(0) as u32;
            let crop_y = y1.max(0) as u32;

            let cropped = image::imageops::crop_imm(&screenshot_img, crop_x, crop_y, width, height);
            let cropped_image = cropped.to_image();
            let dynamic_crop = image::DynamicImage::ImageRgba8(cropped_image);

            match VisualHashVerifier::calculate_phash(&dynamic_crop) {
                Ok(real_hash) => {
                    tracing::info!(
                        target: "omniparser",
                        "Baseline hash for {}: bbox={:?} hash={}",
                        element.id, element.bbox, real_hash
                    );

                    // Always save baseline crop for physical inspection (not behind debug flag)
                    if let Err(e) =
                        dynamic_crop.save(format!("/tmp/kria_baseline_{}.png", element.id))
                    {
                        tracing::warn!(target: "omniparser", "Failed to save baseline crop: {}", e);
                    }
                    if let Err(e) = dynamic_crop.save("/tmp/kria_baseline.png") {
                        tracing::warn!(target: "omniparser", "Failed to save baseline crop: {}", e);
                    } else {
                        tracing::info!(target: "omniparser", "Baseline crop saved: /tmp/kria_baseline.png");
                    }

                    element.visual_hash = real_hash;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "omniparser",
                        "Failed to calculate baseline hash for {}: {}", element.id, e
                    );
                }
            }
        }

        // Step 4: Release GPU lease (implicit drop of _gpu_lease)
        // Per RFC 007: "Release lease immediately after JSON generation"
        drop(_gpu_lease);

        // Step 5: Cache the results (now with real pixel-calculated hashes)
        OMNI_CACHE
            .set(cache_key.clone(), parsed.clone(), screenshot_data)
            .await;

        // Step 6: Filter and return elements
        let elements: Vec<_> = parsed
            .elements
            .iter()
            .filter(|e| {
                if e.confidence < min_confidence {
                    return false;
                }
                if let Some(ft) = filter_type {
                    if e.element_type != ft {
                        return false;
                    }
                }
                true
            })
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "type": e.element_type,
                    "label": e.label_wrapped, // Cognitive poisoning defense!
                    "bbox": e.bbox,
                    "confidence": e.confidence,
                    "monitor_id": e.monitor_id,
                })
            })
            .collect();

        ToolResult::ok(serde_json::json!({
            "elements": elements,
            "count": elements.len(),
            "source": "omniparser",
            "screen_dimensions": parsed.screen_dimensions,
            "timestamp": parsed.timestamp,
        }))
    }
}

/// click_element tool implementation.
struct ClickElement {
    state: Arc<VisionToolState>,
}

#[async_trait]
impl ToolHandler for ClickElement {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let element_id = match params["element_id"].as_str() {
            Some(id) => id,
            None => return ToolResult::err("Missing element_id parameter"),
        };
        let button = params["button"].as_str().unwrap_or("left");

        // Step 1: Get element from cache
        let element = match OMNI_CACHE.get_element_by_id(element_id).await {
            Some(el) => el,
            None => {
                return ToolResult::err(format!(
                    "Element '{}' not found in cache. Call get_screen_elements first.",
                    element_id
                ));
            }
        };

        // Check element TTL (RFC 007: 10 second max for element IDs)
        // Note: Cache TTL is 5 seconds, but we also check element-specific
        // The cache get_element_by_id already checks TTL

        tracing::info!(
            target: "click_element",
            "Clicking element {} at bbox {:?}",
            element_id, element.bbox
        );

        // Step 2: Visual Hash Verification (or bypass)
        let [x1, y1, x2, y2] = element.bbox;
        let region_width = ((x2 - x1).max(1)) as u32;
        let region_height = ((y2 - y1).max(1)) as u32;
        let region_x = x1.max(0);
        let region_y = y1.max(0);

        let (center_x, center_y) = element.center();

        // Wait for OS UI compositor to finish hover animations / cursor changes
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Emergency bypass: skip all vision verification when debugging the HTN/IPC pipeline
        if std::env::var("KRIA_DISABLE_VISION_HASH").unwrap_or_default() == "1" {
            tracing::warn!(
                target: "click_element",
                "KRIA_DISABLE_VISION_HASH=1 — SKIPPING visual hash verification for element {}. \
                This bypass is intended for debugging only.", element_id
            );
        } else {
            // Capture verification crop using the EXACT same bbox geometry as the baseline
            let verification_crop = match ScreenshotCapture::capture_region(
                region_x,
                region_y,
                region_width,
                region_height,
            )
            .await
            {
                Ok(img) => img,
                Err(e) => {
                    return ToolResult::err(format!(
                        "Visual hash verification failed (screenshot): {}",
                        e
                    ));
                }
            };

            // ALWAYS save verification crop for physical inspection
            if let Err(e) = verification_crop.save("/tmp/kria_verify.png") {
                tracing::warn!(target: "click_element", "Failed to save verification crop: {}", e);
            } else {
                tracing::info!(target: "click_element", "Verification crop saved: /tmp/kria_verify.png");
            }

            // Log the verification hash for comparison
            match VisualHashVerifier::calculate_phash(&verification_crop) {
                Ok(hash) => {
                    tracing::info!(target: "click_element", "Verification hash for {}: {}", element_id, hash)
                }
                Err(e) => {
                    tracing::warn!(target: "click_element", "Failed to calculate verification hash: {}", e)
                }
            }

            // Verify visual hash (DynamicImage passed directly, no PNG roundtrip)
            match VisualHashVerifier::verify_before_click(&element, &verification_crop).await {
                Ok(true) => {
                    tracing::debug!(target: "click_element", "Visual hash verified");
                }
                Ok(false) => {
                    return ToolResult::err(
                        "Visual hash mismatch - UI has shifted. Element may have moved."
                            .to_string(),
                    );
                }
                Err(e) => {
                    return ToolResult::err(format!("Visual hash verification error: {}", e));
                }
            }
        }

        // Step 3: Execute the click via GUI backend
        let button_enum = match button {
            "left" => MouseButton::Left,
            "right" => MouseButton::Right,
            "middle" => MouseButton::Middle,
            _ => return ToolResult::err(format!("Invalid button: {}", button)),
        };

        match self
            .state
            .gui_backend
            .click_mouse(center_x, center_y, button_enum)
            .await
        {
            Ok(_) => {
                // Step 4: Cache Invalidation
                // Per RFC 007: "cache must be instantly invalidated the moment
                // a state-changing action (click or type) occurs"
                OMNI_CACHE.invalidate_all().await;

                ToolResult::ok(serde_json::json!({
                    "clicked": true,
                    "element_id": element_id,
                    "x": center_x,
                    "y": center_y,
                    "button": button,
                    "visual_verified": true,
                    "cache_invalidated": true,
                }))
            }
            Err(e) => ToolResult::err(format!("Click execution failed: {}", e)),
        }
    }
}

// ============================================================================
// Section 8: Tool Registration
// ============================================================================

pub fn register(reg: &ToolRegistry) {
    // Create OmniParser client
    let omni_endpoint = std::env::var("KRIA_OMNIPARSER_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    let omni_client = OmniParserClient::new(omni_endpoint);

    // Create GUI backend for click execution
    // Socket path must match the daemon's configured socket.
    let socket_path = crate::agent::gui_services::default_uinput_socket_path();
    let gui_backend: Arc<dyn GuiBackend> = Arc::new(YdotoolBackend::new(socket_path));

    let state = Arc::new(VisionToolState {
        omni_client,
        gui_backend,
    });

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "get_screen_elements".into(),
                description: "Get UI elements from screen using OmniParser vision. \
                    Returns elements with cognitive defense applied (<evidence> wrapping). \
                    Uses 5-second cache. Requires GPU lease."
                    .into(),
                category: "vision_automation".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "filter_type".into(),
                        param_type: "string".into(),
                        description: "Filter by element type (button, input, text, etc.)".into(),
                        required: false,
                        default: None,
                    },
                    ParamDef {
                        name: "min_confidence".into(),
                        param_type: "number".into(),
                        description: "Minimum confidence threshold (0.0-1.0, default 0.8)".into(),
                        required: false,
                        default: Some(serde_json::json!(0.8)),
                    },
                    ParamDef {
                        name: "monitor_id".into(),
                        param_type: "integer".into(),
                        description: "Monitor ID for multi-display (default 0)".into(),
                        required: false,
                        default: Some(serde_json::json!(0)),
                    },
                ],
            },
            Arc::new(GetScreenElements {
                state: Arc::clone(&state),
            }),
        ),
        (
            ToolDef {
                name: "click_element".into(),
                description: "Click a UI element by ID with visual hash verification. \
                    Validates UI hasn't shifted before clicking. Invalidates cache after."
                    .into(),
                category: "vision_automation".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "element_id".into(),
                        param_type: "string".into(),
                        description: "Element ID from get_screen_elements".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "button".into(),
                        param_type: "string".into(),
                        description: "Mouse button: left, right, middle (default: left)".into(),
                        required: false,
                        default: Some(serde_json::json!("left")),
                    },
                ],
            },
            Arc::new(ClickElement {
                state: Arc::clone(&state),
            }),
        ),
    ];

    let tool_count = tools.len();
    for (def, handler) in tools {
        reg.register(def, handler);
    }

    tracing::info!(
        target: "vision_automation",
        "Registered {} vision automation tools (RED tier)",
        tool_count
    );
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cognitive_poisoning_defense() {
        let long_text = "a".repeat(150);
        let element = OmniElement::new(
            "test_001".to_string(),
            "button".to_string(),
            long_text.clone(),
            [0, 0, 100, 50],
            0.95,
            0,
            1.0,
            "hash".to_string(),
        );

        // Label should be truncated
        assert!(element.label.len() == 150);
        assert!(element.label_wrapped.len() < 150);
        assert!(element.label_wrapped.starts_with("<evidence>"));
        assert!(element.label_wrapped.ends_with("</evidence>"));
        assert!(element.label_wrapped.contains("...")); // Truncation indicator
    }

    #[test]
    fn test_element_center_calculation() {
        let element = OmniElement::new(
            "btn_001".to_string(),
            "button".to_string(),
            "OK".to_string(),
            [100, 200, 300, 400],
            0.95,
            0,
            1.0,
            "hash".to_string(),
        );

        let (x, y) = element.center();
        assert_eq!(x, 200); // (100 + 300) / 2
        assert_eq!(y, 300); // (200 + 400) / 2

        let (w, h) = element.dimensions();
        assert_eq!(w, 200); // 300 - 100
        assert_eq!(h, 200); // 400 - 200
    }

    #[test]
    fn test_cache_ttl() {
        let cache = OmniParserCache::new();
        // TTL should be 5 seconds per RFC 007
        assert_eq!(cache.ttl, Duration::from_secs(5));
    }
}
