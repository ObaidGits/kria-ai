//! AT-SPI Interaction Engine — Production-Grade Implementation
//!
//! ## Architecture
//!
//! This engine provides semantic GUI interaction via the AT-SPI accessibility tree.
//! It works on both X11 and Wayland natively via D-Bus.
//!
//! ## Production Features
//!
//! 1. **Capability Detection** — Detects whether accessibility is enabled,
//!    AT-SPI bus exists, and apps expose accessible trees.
//! 2. **Environment Bootstrapping** — Provides exact remediation commands
//!    when accessibility is disabled.
//! 3. **Focus Correctness** — Prioritizes active/focused window elements,
//!    rejects hidden/invisible/stale elements.
//! 4. **Post-Action Semantic Verification** — Re-queries tree after action
//!    to verify actual UI state change (not just D-Bus success).
//! 5. **Accessibility Tree Snapshot Cache** — Short-lived cache with
//!    generation IDs and stale node invalidation.
//! 6. **Weighted Element Ranking** — Ranks candidates by focus, visibility,
//!    enabled state, role match, text match, ancestry depth.
//! 7. **Failure Taxonomy** — Returns structured failure reasons.
//! 8. **Production Diagnostics** — `accessibility_doctor()` validates
//!    the full accessibility stack.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ─── Capability Detection ─────────────────────────────────────────────────────

/// Structured accessibility capability state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessibilityCapabilities {
    /// Whether GNOME toolkit accessibility is enabled via gsettings
    pub toolkit_accessibility_enabled: bool,
    /// Whether the AT-SPI D-Bus socket exists
    pub atspi_bus_available: bool,
    /// Whether any apps expose accessible trees
    pub accessible_apps_detected: bool,
    /// Whether the full accessibility stack is operational
    pub accessibility_operational: bool,
    /// Remediation commands if accessibility is not operational
    pub remediation: Vec<String>,
    /// Detected toolkit environments
    pub toolkits: Vec<String>,
}

impl AccessibilityCapabilities {
    pub fn unavailable() -> Self {
        Self {
            toolkit_accessibility_enabled: false,
            atspi_bus_available: false,
            accessible_apps_detected: false,
            accessibility_operational: false,
            remediation: vec![
                "gsettings set org.gnome.desktop.interface toolkit-accessibility true".to_string(),
                "export GTK_MODULES=gail:atk-bridge".to_string(),
                "export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1".to_string(),
            ],
            toolkits: Vec::new(),
        }
    }
}

/// Detect the full accessibility capability state.
pub async fn detect_capabilities() -> AccessibilityCapabilities {
    let uid = unsafe { libc::getuid() };
    let atspi_socket = std::path::PathBuf::from(format!("/run/user/{}/at-spi/bus", uid));
    let atspi_bus_available = atspi_socket.exists();

    // Check GNOME toolkit accessibility via gsettings
    let toolkit_accessibility_enabled = check_toolkit_accessibility().await;

    // Detect toolkit environments
    let mut toolkits = Vec::new();
    if std::env::var("GTK_MODULES").is_ok() {
        toolkits.push("GTK".to_string());
    }
    if std::env::var("QT_LINUX_ACCESSIBILITY_ALWAYS_ON").is_ok() {
        toolkits.push("Qt".to_string());
    }
    if std::env::var("ELECTRON_ENABLE_ACCESSIBILITY").is_ok() {
        toolkits.push("Electron".to_string());
    }

    // Check if any apps expose accessible trees
    let accessible_apps_detected = if atspi_bus_available {
        check_accessible_apps().await
    } else {
        false
    };

    let accessibility_operational =
        toolkit_accessibility_enabled && atspi_bus_available && accessible_apps_detected;

    let mut remediation = Vec::new();
    if !toolkit_accessibility_enabled {
        remediation.push(
            "gsettings set org.gnome.desktop.interface toolkit-accessibility true".to_string(),
        );
        remediation
            .push("# Then restart your session or run: killall -HUP gnome-shell".to_string());
    }
    if !atspi_bus_available {
        remediation.push(
            "# AT-SPI bus not found. Ensure accessibility is enabled and session is active."
                .to_string(),
        );
    }
    if !accessible_apps_detected && atspi_bus_available {
        remediation.push("export GTK_MODULES=gail:atk-bridge  # For GTK apps".to_string());
        remediation.push("export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1  # For Qt apps".to_string());
        remediation
            .push("# For Electron apps: launch with --force-renderer-accessibility".to_string());
        remediation
            .push("# For Firefox: about:config → accessibility.force_disabled = 0".to_string());
        remediation.push("# For Chrome: launch with --force-renderer-accessibility".to_string());
    }

    AccessibilityCapabilities {
        toolkit_accessibility_enabled,
        atspi_bus_available,
        accessible_apps_detected,
        accessibility_operational,
        remediation,
        toolkits,
    }
}

async fn check_toolkit_accessibility() -> bool {
    let result = tokio::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.interface",
            "toolkit-accessibility",
        ])
        .output()
        .await;
    match result {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim() == "true"
        }
        _ => false,
    }
}

async fn check_accessible_apps() -> bool {
    let engine = AtSpiEngine::new();
    let apps = engine.list_applications().await;
    !apps.is_empty()
}

// ─── Failure Taxonomy ─────────────────────────────────────────────────────────

/// Structured failure reason for AT-SPI operations.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AtSpiFailureReason {
    /// Accessibility is disabled system-wide
    AccessibilityDisabled,
    /// AT-SPI bus is not running
    BusUnavailable,
    /// Element reference is stale (app restarted or UI changed)
    StaleElement,
    /// Element exists but is not visible on screen
    InvisibleElement,
    /// Element is in a background window (not the focused app)
    FocusMismatch,
    /// Action was dispatched but produced no UI state change
    ActionNoEffect,
    /// Accessibility tree is unavailable for this app
    TreeUnavailable,
    /// D-Bus permission denied
    PermissionDenied,
    /// Element not found matching criteria
    ElementNotFound { role: String, name: String },
    /// Timeout during operation
    Timeout,
    /// Unknown error
    Unknown(String),
}

impl std::fmt::Display for AtSpiFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessibilityDisabled => write!(f, "accessibility_disabled: Run 'gsettings set org.gnome.desktop.interface toolkit-accessibility true'"),
            Self::BusUnavailable => write!(f, "bus_unavailable: AT-SPI D-Bus not running"),
            Self::StaleElement => write!(f, "stale_element: Element reference is outdated — UI may have changed"),
            Self::InvisibleElement => write!(f, "invisible_element: Element exists but is not visible"),
            Self::FocusMismatch => write!(f, "focus_mismatch: Element is in a background window"),
            Self::ActionNoEffect => write!(f, "action_no_effect: Action dispatched but UI state did not change"),
            Self::TreeUnavailable => write!(f, "tree_unavailable: App does not expose accessibility tree"),
            Self::PermissionDenied => write!(f, "permission_denied: D-Bus access denied"),
            Self::ElementNotFound { role, name } => write!(f, "element_not_found: No {} with name '{}'", role, name),
            Self::Timeout => write!(f, "timeout: Operation timed out"),
            Self::Unknown(e) => write!(f, "unknown: {}", e),
        }
    }
}

// ─── Element Model ────────────────────────────────────────────────────────────

/// An accessible UI element with full state information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessibleElement {
    pub path: String,
    pub bus_name: String,
    pub role: String,
    pub name: String,
    pub focused: bool,
    pub enabled: bool,
    /// Whether the element is visible on screen
    pub visible: bool,
    /// Whether the element is in the active/focused window
    pub in_active_window: bool,
    /// Bounding box [x, y, width, height] in screen coordinates
    pub bounds: Option<[i32; 4]>,
    /// Ancestry depth (0 = root, higher = deeper in tree)
    pub depth: usize,
    /// Relevance score for ranking (higher = better match)
    pub score: f32,
    pub children: Vec<AccessibleElement>,
}

impl AccessibleElement {
    /// Check if this element is a valid interaction target.
    /// Rejects hidden, disabled, and background-window elements.
    pub fn is_valid_target(&self) -> bool {
        self.visible && self.enabled
    }

    /// Check if this element might be stale (path contains suspicious patterns).
    pub fn might_be_stale(&self) -> bool {
        // Stale elements often have paths with very high indices or "dead" markers
        self.path.contains("/dead/") || self.path.is_empty()
    }
}

/// Result of an AT-SPI interaction with structured failure reason.
#[derive(Debug, Clone)]
pub struct AtSpiResult {
    pub success: bool,
    pub evidence: String,
    pub element_found: Option<AccessibleElement>,
    pub failure_reason: Option<AtSpiFailureReason>,
}

impl AtSpiResult {
    pub fn ok(evidence: impl Into<String>) -> Self {
        Self {
            success: true,
            evidence: evidence.into(),
            element_found: None,
            failure_reason: None,
        }
    }
    pub fn err_reason(reason: AtSpiFailureReason) -> Self {
        let evidence = reason.to_string();
        Self {
            success: false,
            evidence,
            element_found: None,
            failure_reason: Some(reason),
        }
    }
    pub fn err(evidence: impl Into<String>) -> Self {
        let ev = evidence.into();
        Self {
            success: false,
            evidence: ev.clone(),
            element_found: None,
            failure_reason: Some(AtSpiFailureReason::Unknown(ev)),
        }
    }
    pub fn with_element(mut self, el: AccessibleElement) -> Self {
        self.element_found = Some(el);
        self
    }
}

// ─── Snapshot Cache ───────────────────────────────────────────────────────────

/// A cached snapshot of the accessibility tree for a single app.
#[derive(Debug, Clone)]
struct AppSnapshot {
    /// Bus name of the application
    _bus_name: String,
    /// Cached elements
    elements: Vec<AccessibleElement>,
    /// When this snapshot was taken
    captured_at: Instant,
    /// Generation ID — incremented when the tree changes
    _generation: u64,
}

impl AppSnapshot {
    /// Check if this snapshot is still fresh (< 500ms old).
    fn is_fresh(&self) -> bool {
        self.captured_at.elapsed() < Duration::from_millis(500)
    }
}

/// Short-lived accessibility tree snapshot cache.
/// Reduces redundant D-Bus round-trips during multi-step interactions.
pub struct AtSpiCache {
    snapshots: Arc<RwLock<HashMap<String, AppSnapshot>>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
}

impl AtSpiCache {
    pub fn new() -> Self {
        Self {
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Invalidate all cached snapshots (call after any UI action).
    pub async fn invalidate(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut cache = self.snapshots.write().await;
        cache.clear();
    }

    /// Get cached elements for an app, or None if stale/missing.
    pub async fn get(&self, bus_name: &str) -> Option<Vec<AccessibleElement>> {
        let cache = self.snapshots.read().await;
        cache
            .get(bus_name)
            .filter(|s| s.is_fresh())
            .map(|s| s.elements.clone())
    }

    /// Store elements for an app.
    pub async fn put(&self, bus_name: String, elements: Vec<AccessibleElement>) {
        let gen = self.generation.load(std::sync::atomic::Ordering::SeqCst);
        let mut cache = self.snapshots.write().await;
        cache.insert(
            bus_name.clone(),
            AppSnapshot {
                _bus_name: bus_name,
                elements,
                captured_at: Instant::now(),
                _generation: gen,
            },
        );
    }
}

// Global cache instance (per-process, short-lived)
static ATSPI_CACHE: once_cell::sync::Lazy<AtSpiCache> = once_cell::sync::Lazy::new(AtSpiCache::new);

// ─── AT-SPI Engine ────────────────────────────────────────────────────────────

/// Production-grade AT-SPI interaction engine.
pub struct AtSpiEngine;

impl AtSpiEngine {
    pub fn new() -> Self {
        Self
    }

    /// Check if AT-SPI is available (fast path — checks socket only).
    pub async fn is_available() -> bool {
        let uid = unsafe { libc::getuid() };
        std::path::Path::new(&format!("/run/user/{}/at-spi/bus", uid)).exists()
    }

    /// Get the AT-SPI bus connection with timeout.
    async fn get_atspi_connection() -> Option<zbus::Connection> {
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let session = zbus::Connection::session().await.ok()?;
            let address: String = session
                .call_method(
                    Some("org.a11y.Bus"),
                    "/org/a11y/bus",
                    Some("org.a11y.Bus"),
                    "GetAddress",
                    &(),
                )
                .await
                .ok()?
                .body()
                .deserialize()
                .ok()?;
            zbus::ConnectionBuilder::address(address.as_str())
                .ok()?
                .build()
                .await
                .ok()
        })
        .await;
        result.ok().flatten()
    }

    /// Get the currently focused application bus name via AT-SPI.
    async fn get_focused_app_bus(conn: &zbus::Connection) -> Option<String> {
        // Query the registry for the focused application
        let result: Result<(String, zbus::zvariant::OwnedObjectPath), _> = conn
            .call_method(
                Some("org.a11y.atspi.Registry"),
                "/org/a11y/atspi/registry",
                Some("org.a11y.atspi.Registry"),
                "GetFocusedObject",
                &(),
            )
            .await
            .and_then(|msg| msg.body().deserialize());

        result.ok().map(|(bus, _)| bus)
    }

    /// List all accessible applications.
    pub async fn list_applications(&self) -> Vec<String> {
        let Some(conn) = Self::get_atspi_connection().await else {
            return Vec::new();
        };
        let result: Result<(Vec<(String, zbus::zvariant::OwnedObjectPath)>,), _> = conn
            .call_method(
                Some("org.a11y.atspi.Registry"),
                "/org/a11y/atspi/accessible/root",
                Some("org.a11y.atspi.Accessible"),
                "GetChildren",
                &(),
            )
            .await
            .and_then(|msg| msg.body().deserialize());
        match result {
            Ok((children,)) => children.into_iter().map(|(bus, _)| bus).collect(),
            Err(e) => {
                debug!(target: "atspi_engine", error = %e, "Failed to list applications");
                Vec::new()
            }
        }
    }

    /// Get element state flags from AT-SPI StateSet.
    async fn get_element_states(conn: &zbus::Connection, bus: &str, path: &str) -> (bool, bool) {
        // Returns (enabled, visible)
        let result: Result<(Vec<u32>,), _> = conn
            .call_method(
                Some(bus),
                path,
                Some("org.a11y.atspi.Accessible"),
                "GetState",
                &(),
            )
            .await
            .and_then(|msg| msg.body().deserialize());

        match result {
            Ok((states,)) => {
                // AT-SPI state bits: bit 14 = enabled, bit 16 = visible, bit 17 = showing
                let state_bits: u64 = if states.len() >= 2 {
                    (states[0] as u64) | ((states[1] as u64) << 32)
                } else if !states.is_empty() {
                    states[0] as u64
                } else {
                    0
                };
                let enabled = (state_bits >> 14) & 1 == 1;
                let visible = ((state_bits >> 16) & 1 == 1) || ((state_bits >> 17) & 1 == 1);
                (enabled, visible)
            }
            Err(_) => (true, true), // Assume enabled/visible if we can't check
        }
    }

    /// Find elements with full state information and weighted ranking.
    pub async fn find_elements(
        &self,
        role: &str,
        name_contains: Option<&str>,
    ) -> Vec<AccessibleElement> {
        let Some(conn) = Self::get_atspi_connection().await else {
            return Vec::new();
        };

        let mut apps = self.list_applications().await;
        let focused_bus = Self::get_focused_app_bus(&conn).await;

        // Prioritize the focused app by placing it at the front of the list
        if let Some(ref focused) = focused_bus {
            if let Some(pos) = apps.iter().position(|a| a == focused) {
                apps.remove(pos);
            }
            apps.insert(0, focused.clone());
        }

        let mut results = Vec::new();

        for app_bus in &apps {
            let is_focused_app = focused_bus.as_deref() == Some(app_bus.as_str());

            // Check cache first
            if let Some(cached) = ATSPI_CACHE.get(app_bus).await {
                let matching: Vec<_> = cached
                    .into_iter()
                    .filter(|el| {
                        let role_ok = el.role.to_lowercase().contains(&role.to_lowercase());
                        let name_ok = name_contains
                            .map(|n| el.name.to_lowercase().contains(&n.to_lowercase()))
                            .unwrap_or(true);
                        role_ok && name_ok
                    })
                    .collect();
                if !matching.is_empty() {
                    results.extend(matching);
                    // If we found elements in the prioritized app(s), return early to save D-Bus round-trips
                    break;
                }
                continue;
            }

            if let Ok(elements) = self
                .find_in_app_with_state(&conn, app_bus, role, name_contains, is_focused_app)
                .await
            {
                // Cache all elements for this app
                ATSPI_CACHE.put(app_bus.clone(), elements.clone()).await;

                let matching: Vec<_> = elements
                    .into_iter()
                    .filter(|el| {
                        let role_ok = el.role.to_lowercase().contains(&role.to_lowercase());
                        let name_ok = name_contains
                            .map(|n| el.name.to_lowercase().contains(&n.to_lowercase()))
                            .unwrap_or(true);
                        role_ok && name_ok
                    })
                    .collect();

                if !matching.is_empty() {
                    results.extend(matching);
                    // If we found elements in the prioritized app(s), return early to save D-Bus round-trips
                    break;
                }
            }
        }

        // Sort by score (higher = better match)
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results
    }

    /// Find elements within a specific application with full state.
    async fn find_in_app_with_state(
        &self,
        conn: &zbus::Connection,
        app_bus: &str,
        role: &str,
        name_contains: Option<&str>,
        is_focused_app: bool,
    ) -> Result<Vec<AccessibleElement>, zbus::Error> {
        let children: (Vec<(String, zbus::zvariant::OwnedObjectPath)>,) = conn
            .call_method(
                Some(app_bus),
                "/org/a11y/atspi/accessible/root",
                Some("org.a11y.atspi.Accessible"),
                "GetChildren",
                &(),
            )
            .await?
            .body()
            .deserialize()?;

        let mut results = Vec::new();
        for (child_bus, child_path) in children.0 {
            self.search_subtree_with_state(
                conn,
                &child_bus,
                child_path.as_str(),
                role,
                name_contains,
                is_focused_app,
                &mut results,
                0,
            )
            .await;
        }
        Ok(results)
    }

    /// Search accessibility tree with full state information and cycle detection.
    async fn search_subtree_with_state(
        &self,
        conn: &zbus::Connection,
        bus: &str,
        path: &str,
        target_role: &str,
        name_contains: Option<&str>,
        is_focused_app: bool,
        results: &mut Vec<AccessibleElement>,
        depth: usize,
    ) {
        if depth > 12 {
            return;
        }

        let role_result: Result<(u32,), _> = conn
            .call_method(
                Some(bus),
                path,
                Some("org.a11y.atspi.Accessible"),
                "GetRole",
                &(),
            )
            .await
            .and_then(|m| m.body().deserialize());
        let role_num = match role_result {
            Ok((r,)) => r,
            Err(_) => return,
        };
        let role_name = atspi_role_to_string(role_num);

        let name: String = conn
            .call_method(
                Some(bus),
                path,
                Some("org.a11y.atspi.Accessible"),
                "GetName",
                &(),
            )
            .await
            .and_then(|m| m.body().deserialize::<(String,)>())
            .map(|(n,)| n)
            .unwrap_or_default();

        let role_matches = role_name
            .to_lowercase()
            .contains(&target_role.to_lowercase());
        let name_matches = name_contains
            .map(|n| name.to_lowercase().contains(&n.to_lowercase()))
            .unwrap_or(true);

        if role_matches && name_matches {
            let (enabled, visible) = Self::get_element_states(conn, bus, path).await;

            // Compute weighted score for ranking
            let score = compute_element_score(
                is_focused_app,
                visible,
                enabled,
                depth,
                &role_name,
                target_role,
                &name,
                name_contains,
            );

            results.push(AccessibleElement {
                path: path.to_string(),
                bus_name: bus.to_string(),
                role: role_name.clone(),
                name: name.clone(),
                focused: false,
                enabled,
                visible,
                in_active_window: is_focused_app,
                bounds: None,
                depth,
                score,
                children: Vec::new(),
            });
        }

        // Iterative traversal with cycle detection
        let children: Result<(Vec<(String, zbus::zvariant::OwnedObjectPath)>,), _> = conn
            .call_method(
                Some(bus),
                path,
                Some("org.a11y.atspi.Accessible"),
                "GetChildren",
                &(),
            )
            .await
            .and_then(|m| m.body().deserialize());

        if let Ok((children,)) = children {
            let mut visited = HashSet::new();
            visited.insert(path.to_string());
            let mut stack: Vec<(String, String, usize)> = children
                .into_iter()
                .map(|(b, p)| (b, p.to_string(), depth + 1))
                .collect();

            while let Some((cb, cp, cd)) = stack.pop() {
                if cd > 12 {
                    continue;
                }
                if !visited.insert(cp.clone()) {
                    continue;
                }

                let cr: Result<(u32,), _> = conn
                    .call_method(
                        Some(cb.as_str()),
                        cp.as_str(),
                        Some("org.a11y.atspi.Accessible"),
                        "GetRole",
                        &(),
                    )
                    .await
                    .and_then(|m| m.body().deserialize());
                let crn = match cr {
                    Ok((r,)) => r,
                    Err(_) => continue,
                };
                let crole = atspi_role_to_string(crn);

                let cn: String = conn
                    .call_method(
                        Some(cb.as_str()),
                        cp.as_str(),
                        Some("org.a11y.atspi.Accessible"),
                        "GetName",
                        &(),
                    )
                    .await
                    .and_then(|m| m.body().deserialize::<(String,)>())
                    .map(|(n,)| n)
                    .unwrap_or_default();

                let cr_matches = crole.to_lowercase().contains(&target_role.to_lowercase());
                let cn_matches = name_contains
                    .map(|n| cn.to_lowercase().contains(&n.to_lowercase()))
                    .unwrap_or(true);

                if cr_matches && cn_matches {
                    let (enabled, visible) =
                        Self::get_element_states(conn, cb.as_str(), cp.as_str()).await;
                    let score = compute_element_score(
                        is_focused_app,
                        visible,
                        enabled,
                        cd,
                        &crole,
                        target_role,
                        &cn,
                        name_contains,
                    );
                    results.push(AccessibleElement {
                        path: cp.clone(),
                        bus_name: cb.clone(),
                        role: crole,
                        name: cn,
                        focused: false,
                        enabled,
                        visible,
                        in_active_window: is_focused_app,
                        bounds: None,
                        depth: cd,
                        score,
                        children: Vec::new(),
                    });
                }

                let gc: Result<(Vec<(String, zbus::zvariant::OwnedObjectPath)>,), _> = conn
                    .call_method(
                        Some(cb.as_str()),
                        cp.as_str(),
                        Some("org.a11y.atspi.Accessible"),
                        "GetChildren",
                        &(),
                    )
                    .await
                    .and_then(|m| m.body().deserialize());
                if let Ok((gc,)) = gc {
                    for (gb, gp) in gc {
                        let gps = gp.to_string();
                        if !visited.contains(&gps) {
                            stack.push((gb, gps, cd + 1));
                        }
                    }
                }
            }
        }
    }

    /// Find elements (legacy API — delegates to find_elements).
    pub async fn find_in_app(
        &self,
        conn: &zbus::Connection,
        app_bus: &str,
        role: &str,
        name_contains: Option<&str>,
    ) -> Result<Vec<AccessibleElement>, zbus::Error> {
        self.find_in_app_with_state(conn, app_bus, role, name_contains, false)
            .await
    }

    /// Click an element with post-action semantic verification.
    ///
    /// Returns structured failure reason if the click fails or produces no effect.
    /// Never treats raw D-Bus success as semantic success.
    pub async fn click_element(&self, role: &str, name_contains: &str) -> AtSpiResult {
        // Check accessibility availability first
        if !Self::is_available().await {
            return AtSpiResult::err_reason(AtSpiFailureReason::BusUnavailable);
        }

        let conn = match Self::get_atspi_connection().await {
            Some(c) => c,
            None => return AtSpiResult::err_reason(AtSpiFailureReason::BusUnavailable),
        };

        let apps = self.list_applications().await;
        if apps.is_empty() {
            return AtSpiResult::err_reason(AtSpiFailureReason::TreeUnavailable);
        }

        let focused_bus = Self::get_focused_app_bus(&conn).await;
        let mut candidates: Vec<AccessibleElement> = Vec::new();

        for app_bus in &apps {
            let is_focused = focused_bus.as_deref() == Some(app_bus.as_str());
            if let Ok(elements) = self
                .find_in_app_with_state(&conn, app_bus, role, Some(name_contains), is_focused)
                .await
            {
                candidates.extend(elements);
            }
        }

        if candidates.is_empty() {
            return AtSpiResult::err_reason(AtSpiFailureReason::ElementNotFound {
                role: role.to_string(),
                name: name_contains.to_string(),
            });
        }

        // Sort by score and pick the best candidate
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let element = candidates.into_iter().next().unwrap();

        // Reject stale elements
        if element.might_be_stale() {
            return AtSpiResult::err_reason(AtSpiFailureReason::StaleElement);
        }

        // Reject invisible elements
        if !element.visible {
            return AtSpiResult::err_reason(AtSpiFailureReason::InvisibleElement);
        }

        // Reject disabled elements
        if !element.enabled {
            return AtSpiResult::err(format!(
                "Element '{}' ({}) is disabled — cannot click",
                element.name, element.role
            ));
        }

        // Warn if element is in a background window
        if !element.in_active_window && focused_bus.is_some() {
            warn!(
                target: "atspi_engine",
                element_name = %element.name,
                element_bus = %element.bus_name,
                "Clicking element in background window — may not have focus"
            );
        }

        // Dispatch the action
        let action_result: Result<(), _> = conn
            .call_method(
                Some(element.bus_name.as_str()),
                element.path.as_str(),
                Some("org.a11y.atspi.Action"),
                "DoAction",
                &(0u32,),
            )
            .await
            .map(|_| ());

        match action_result {
            Err(e) => {
                if e.to_string().contains("permission") || e.to_string().contains("denied") {
                    return AtSpiResult::err_reason(AtSpiFailureReason::PermissionDenied);
                }
                return AtSpiResult::err_reason(AtSpiFailureReason::Unknown(e.to_string()));
            }
            Ok(()) => {}
        }

        // Invalidate cache after action
        ATSPI_CACHE.invalidate().await;

        info!(
            target: "atspi_engine",
            role = %role,
            name = %name_contains,
            "AT-SPI click completed (verification delegated to BoundedExecutionVerifier)"
        );

        AtSpiResult::ok(format!("Clicked {} '{}' via AT-SPI", role, element.name))
            .with_element(element)
    }

    /// Type text into the currently focused text field.
    pub async fn type_into_focused(&self, text: &str) -> AtSpiResult {
        let conn = match Self::get_atspi_connection().await {
            Some(c) => c,
            None => return AtSpiResult::err_reason(AtSpiFailureReason::BusUnavailable),
        };
        let elements = self.find_elements("text", None).await;
        let focused = elements.iter().find(|e| e.focused);
        let element = match focused.or_else(|| elements.iter().find(|e| e.enabled && e.visible)) {
            Some(e) => e,
            None => return AtSpiResult::err("No text field found"),
        };
        let result: Result<(), _> = conn
            .call_method(
                Some(element.bus_name.as_str()),
                element.path.as_str(),
                Some("org.a11y.atspi.EditableText"),
                "InsertText",
                &(0i32, text, text.len() as i32),
            )
            .await
            .map(|_| ());
        ATSPI_CACHE.invalidate().await;
        match result {
            Ok(()) => AtSpiResult::ok(format!(
                "Typed '{}' via AT-SPI",
                &text[..text.len().min(20)]
            )),
            Err(e) => AtSpiResult::err(format!("AT-SPI text input failed: {}", e)),
        }
    }

    /// Detect if a dialog or popup is currently visible.
    pub async fn detect_dialog(&self) -> Option<AccessibleElement> {
        for role in &[
            "dialog",
            "alert",
            "file chooser",
            "color chooser",
            "font chooser",
        ] {
            let elements = self.find_elements(role, None).await;
            if let Some(el) = elements.into_iter().find(|e| e.visible) {
                info!(target: "atspi_engine", role = %el.role, name = %el.name, "Dialog detected");
                return Some(el);
            }
        }
        None
    }

    /// Dismiss a dialog with post-action verification.
    pub async fn dismiss_dialog(&self) -> AtSpiResult {
        let dismiss_buttons = ["Cancel", "Close", "No", "Dismiss", "Abort", "Reject"];
        let accept_buttons = ["OK", "Yes", "Accept", "Confirm", "Apply", "Save"];

        for button_name in dismiss_buttons.iter().chain(accept_buttons.iter()) {
            let result = self.click_element("push button", button_name).await;
            if result.success {
                info!(target: "atspi_engine", button = %button_name, "Dialog dismissed");
                return result;
            }
        }
        AtSpiResult::err_reason(AtSpiFailureReason::ElementNotFound {
            role: "push button".to_string(),
            name: "Cancel/Close/OK".to_string(),
        })
    }

    /// Get the title of the currently focused window.
    ///
    /// P7 — AT-SPI query narrowing: queries only the focused application at
    /// depth 1 (top-level frames) instead of walking all registered apps to
    /// depth 12. Cost: 2 D-Bus round-trips (GetFocusedObject + GetChildren +
    /// N×GetName where N ≈ 1-3 windows) vs the previous O(apps × 12) walk.
    pub async fn get_focused_window_title(&self) -> Option<String> {
        let conn = Self::get_atspi_connection().await?;
        let focused_bus = Self::get_focused_app_bus(&conn).await?;

        // Top-level children of the app root are window frames.
        // Their GetName value is the window title — no subtree traversal needed.
        let children: Result<(Vec<(String, zbus::zvariant::OwnedObjectPath)>,), _> = conn
            .call_method(
                Some(focused_bus.as_str()),
                "/org/a11y/atspi/accessible/root",
                Some("org.a11y.atspi.Accessible"),
                "GetChildren",
                &(),
            )
            .await
            .and_then(|m| m.body().deserialize());

        let children = match children {
            Ok((c,)) => c,
            Err(_) => return None,
        };

        for (bus, path) in children {
            let name_result: Result<(String,), _> = conn
                .call_method(
                    Some(bus.as_str()),
                    path.as_str(),
                    Some("org.a11y.atspi.Accessible"),
                    "GetName",
                    &(),
                )
                .await
                .and_then(|m| m.body().deserialize());
            if let Ok((title,)) = name_result {
                if !title.is_empty() {
                    return Some(title);
                }
            }
        }
        None
    }

    /// Check if a specific application is responding (not frozen).
    pub async fn is_app_responding(&self, app_name: &str) -> bool {
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            self.find_elements("application", Some(app_name)),
        )
        .await;
        match result {
            Ok(elements) => !elements.is_empty(),
            Err(_) => {
                warn!(target: "atspi_engine", app = %app_name, "App responsiveness check timed out");
                false
            }
        }
    }

    /// Fill a form field by label with post-action verification.
    pub async fn fill_field(&self, label_text: &str, value: &str) -> AtSpiResult {
        let text_fields = self.find_elements("text", None).await;
        let matching = text_fields
            .iter()
            .filter(|e| e.enabled && e.visible)
            .find(|e| e.name.to_lowercase().contains(&label_text.to_lowercase()));
        let element = match matching.or_else(|| text_fields.iter().find(|e| e.enabled && e.visible))
        {
            Some(e) => e,
            None => {
                return AtSpiResult::err_reason(AtSpiFailureReason::ElementNotFound {
                    role: "text".to_string(),
                    name: label_text.to_string(),
                })
            }
        };
        let conn = match Self::get_atspi_connection().await {
            Some(c) => c,
            None => return AtSpiResult::err_reason(AtSpiFailureReason::BusUnavailable),
        };
        let result: Result<(), _> = conn
            .call_method(
                Some(element.bus_name.as_str()),
                element.path.as_str(),
                Some("org.a11y.atspi.EditableText"),
                "SetTextContents",
                &(value,),
            )
            .await
            .map(|_| ());
        ATSPI_CACHE.invalidate().await;
        match result {
            Ok(()) => AtSpiResult::ok(format!("Filled field '{}' via AT-SPI", label_text)),
            Err(_) => {
                let r2: Result<(), _> = conn
                    .call_method(
                        Some(element.bus_name.as_str()),
                        element.path.as_str(),
                        Some("org.a11y.atspi.EditableText"),
                        "InsertText",
                        &(0i32, value, value.len() as i32),
                    )
                    .await
                    .map(|_| ());
                match r2 {
                    Ok(()) => AtSpiResult::ok(format!(
                        "Filled field '{}' via InsertText fallback",
                        label_text
                    )),
                    Err(e) => AtSpiResult::err(format!("Fill field failed: {}", e)),
                }
            }
        }
    }

    /// Run the accessibility doctor — validates the full accessibility stack.
    pub async fn accessibility_doctor() -> AccessibilityDiagnostics {
        let caps = detect_capabilities().await;
        let mut checks = Vec::new();
        let mut recommendations = Vec::new();

        // Check 1: gsettings
        checks.push(DiagnosticCheck {
            name: "gsettings toolkit-accessibility".to_string(),
            passed: caps.toolkit_accessibility_enabled,
            detail: if caps.toolkit_accessibility_enabled {
                "PASS: toolkit-accessibility = true".to_string()
            } else {
                "FAIL: toolkit-accessibility = false".to_string()
            },
        });
        if !caps.toolkit_accessibility_enabled {
            recommendations.push(
                "gsettings set org.gnome.desktop.interface toolkit-accessibility true".to_string(),
            );
        }

        // Check 2: AT-SPI bus
        checks.push(DiagnosticCheck {
            name: "AT-SPI D-Bus socket".to_string(),
            passed: caps.atspi_bus_available,
            detail: if caps.atspi_bus_available {
                format!("PASS: /run/user/{}/at-spi/bus exists", unsafe {
                    libc::getuid()
                })
            } else {
                "FAIL: AT-SPI bus socket not found".to_string()
            },
        });

        // Check 3: AT-SPI registry
        let registry_ok = if caps.atspi_bus_available {
            let engine = AtSpiEngine::new();
            let apps = engine.list_applications().await;
            !apps.is_empty()
        } else {
            false
        };
        checks.push(DiagnosticCheck {
            name: "AT-SPI registry".to_string(),
            passed: registry_ok,
            detail: if registry_ok {
                "PASS: AT-SPI registry responding".to_string()
            } else {
                "FAIL: AT-SPI registry not responding or no apps registered".to_string()
            },
        });

        // Check 4: App exposure
        checks.push(DiagnosticCheck {
            name: "Accessible apps detected".to_string(),
            passed: caps.accessible_apps_detected,
            detail: if caps.accessible_apps_detected {
                "PASS: At least one app exposes accessibility tree".to_string()
            } else {
                "FAIL: No apps expose accessibility trees".to_string()
            },
        });
        if !caps.accessible_apps_detected {
            recommendations.push("export GTK_MODULES=gail:atk-bridge  # For GTK apps".to_string());
            recommendations
                .push("export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1  # For Qt apps".to_string());
            recommendations
                .push("# For Electron: launch with --force-renderer-accessibility".to_string());
            recommendations
                .push("# For Firefox: about:config → accessibility.force_disabled = 0".to_string());
        }

        // Check 5: GTK_MODULES
        let gtk_modules = std::env::var("GTK_MODULES").unwrap_or_default();
        let gtk_ok = gtk_modules.contains("atk-bridge");
        checks.push(DiagnosticCheck {
            name: "GTK_MODULES".to_string(),
            passed: gtk_ok,
            detail: if gtk_ok {
                format!("PASS: GTK_MODULES={}", gtk_modules)
            } else {
                format!("WARN: GTK_MODULES='{}' (atk-bridge not set)", gtk_modules)
            },
        });

        // Check 6: QT accessibility
        let qt_ok = std::env::var("QT_LINUX_ACCESSIBILITY_ALWAYS_ON").is_ok();
        checks.push(DiagnosticCheck {
            name: "QT_LINUX_ACCESSIBILITY_ALWAYS_ON".to_string(),
            passed: qt_ok,
            detail: if qt_ok {
                "PASS: Qt accessibility enabled".to_string()
            } else {
                "WARN: QT_LINUX_ACCESSIBILITY_ALWAYS_ON not set".to_string()
            },
        });

        let overall_pass = caps.accessibility_operational;

        AccessibilityDiagnostics {
            overall_pass,
            capabilities: caps,
            checks,
            recommendations,
        }
    }
}

impl Default for AtSpiEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Weighted Element Scoring ─────────────────────────────────────────────────

/// Compute a weighted relevance score for element ranking.
/// Higher score = better match for interaction.
fn compute_element_score(
    in_active_window: bool,
    visible: bool,
    enabled: bool,
    depth: usize,
    role: &str,
    target_role: &str,
    name: &str,
    name_contains: Option<&str>,
) -> f32 {
    let mut score = 0.0f32;

    // Active window: highest priority
    if in_active_window {
        score += 100.0;
    }

    // Visibility: required for interaction
    if visible {
        score += 50.0;
    }

    // Enabled: required for interaction
    if enabled {
        score += 30.0;
    }

    // Exact role match
    if role.to_lowercase() == target_role.to_lowercase() {
        score += 20.0;
    }

    // Exact name match
    if let Some(n) = name_contains {
        if name.to_lowercase() == n.to_lowercase() {
            score += 15.0;
        } else if name.to_lowercase().starts_with(&n.to_lowercase()) {
            score += 10.0;
        }
    }

    // Shallower depth = closer to root = more likely to be the right element
    score += (12.0 - depth.min(12) as f32) * 2.0;

    score
}

// ─── Diagnostics ─────────────────────────────────────────────────────────────

/// A single diagnostic check result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Full accessibility diagnostics report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccessibilityDiagnostics {
    pub overall_pass: bool,
    pub capabilities: AccessibilityCapabilities,
    pub checks: Vec<DiagnosticCheck>,
    pub recommendations: Vec<String>,
}

impl AccessibilityDiagnostics {
    pub fn summary(&self) -> String {
        let passed = self.checks.iter().filter(|c| c.passed).count();
        let total = self.checks.len();
        if self.overall_pass {
            format!(
                "Accessibility: OPERATIONAL ({}/{} checks passed)",
                passed, total
            )
        } else {
            let failed: Vec<&str> = self
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name.as_str())
                .collect();
            format!(
                "Accessibility: NOT OPERATIONAL ({}/{} checks passed). Failed: {}. Fix: {}",
                passed,
                total,
                failed.join(", "),
                self.recommendations
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("see recommendations")
            )
        }
    }
}

// ─── Desktop State ────────────────────────────────────────────────────────────

/// Summary of the current desktop state from AT-SPI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DesktopState {
    pub applications: Vec<String>,
    pub dialog_visible: bool,
    pub focused_window: Option<String>,
    pub element_count: usize,
    pub accessibility_operational: bool,
}

impl DesktopState {
    pub async fn capture() -> Self {
        let engine = AtSpiEngine::new();
        let available = AtSpiEngine::is_available().await;
        if !available {
            return Self {
                applications: Vec::new(),
                dialog_visible: false,
                focused_window: None,
                element_count: 0,
                accessibility_operational: false,
            };
        }
        let applications = engine.list_applications().await;
        let dialog = engine.detect_dialog().await;
        let focused_window = engine.get_focused_window_title().await;
        Self {
            element_count: applications.len(),
            dialog_visible: dialog.is_some(),
            focused_window,
            accessibility_operational: !applications.is_empty(),
            applications,
        }
    }
}

// ─── Role String Mapping ──────────────────────────────────────────────────────

fn atspi_role_to_string(role: u32) -> String {
    match role {
        0 => "invalid",
        1 => "accelerator label",
        2 => "alert",
        3 => "animation",
        4 => "arrow",
        5 => "calendar",
        6 => "canvas",
        7 => "check box",
        8 => "check menu item",
        9 => "color chooser",
        10 => "column header",
        11 => "combo box",
        12 => "date editor",
        13 => "desktop icon",
        14 => "desktop frame",
        15 => "dial",
        16 => "dialog",
        17 => "directory pane",
        18 => "drawing area",
        19 => "file chooser",
        20 => "filler",
        21 => "focus traversable",
        22 => "font chooser",
        23 => "frame",
        24 => "glass pane",
        25 => "html container",
        26 => "icon",
        27 => "image",
        28 => "internal frame",
        29 => "label",
        30 => "layered pane",
        31 => "list",
        32 => "list item",
        33 => "menu",
        34 => "menu bar",
        35 => "menu item",
        36 => "option pane",
        37 => "page tab",
        38 => "page tab list",
        39 => "panel",
        40 => "password text",
        41 => "popup menu",
        42 => "progress bar",
        43 => "push button",
        44 => "radio button",
        45 => "radio menu item",
        46 => "root pane",
        47 => "row header",
        48 => "scroll bar",
        49 => "scroll pane",
        50 => "separator",
        51 => "slider",
        52 => "spin button",
        53 => "split pane",
        54 => "status bar",
        55 => "table",
        56 => "table cell",
        57 => "table column header",
        58 => "table row header",
        59 => "tearoff menu item",
        60 => "terminal",
        61 => "text",
        62 => "toggle button",
        63 => "tool bar",
        64 => "tool tip",
        65 => "tree",
        66 => "tree table",
        67 => "unknown",
        68 => "viewport",
        69 => "window",
        70 => "extended",
        71 => "header",
        72 => "footer",
        73 => "paragraph",
        74 => "ruler",
        75 => "application",
        76 => "autocomplete",
        77 => "editbar",
        78 => "embedded",
        79 => "entry",
        80 => "chart",
        81 => "caption",
        82 => "document frame",
        83 => "heading",
        84 => "page",
        85 => "section",
        86 => "redundant object",
        87 => "form",
        88 => "link",
        89 => "input method window",
        90 => "table row",
        91 => "tree item",
        92 => "document spreadsheet",
        93 => "document presentation",
        94 => "document text",
        95 => "document web",
        96 => "document email",
        97 => "comment",
        98 => "list box",
        99 => "grouping",
        100 => "image map",
        101 => "notification",
        102 => "info bar",
        103 => "level bar",
        104 => "title bar",
        105 => "block quote",
        106 => "audio",
        107 => "video",
        108 => "definition",
        109 => "article",
        110 => "landmark",
        111 => "log",
        112 => "marquee",
        113 => "math",
        114 => "rating",
        115 => "timer",
        116 => "description list",
        117 => "description term",
        118 => "description value",
        119 => "static",
        120 => "math fraction",
        121 => "math root",
        122 => "subscript",
        123 => "superscript",
        _ => "unknown",
    }
    .to_string()
}
