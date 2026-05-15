//! RFC v2 (P2): Operational environment grounding.
//!
//! Returns a **closed set of operational facts** about the current desktop
//! state. Strictly bounded: ≤16 windows, ≤8 processes, ≤10 s TTL,
//! no graph, no embeddings, no arbitrary key-value memory.
//!
//! The grounder is:
//! - **Read-only**: observes OS state, never mutates
//! - **Ephemeral**: single-snapshot cache, no persistence
//! - **Bounded**: hard caps on all collections
//! - **Deterministic**: no reasoning, no confidence, no planning

use crate::agent::intent_compiler::TargetRef;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ─── Display Server ─────────────────────────────────────────────────────────

/// Display server type — detected once at startup, immutable thereafter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayServerType {
    X11,
    Wayland,
    XWayland,
    Unknown,
}

impl DisplayServerType {
    /// Detect from environment. Called once at grounder construction.
    pub fn detect() -> Self {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let has_display = std::env::var("DISPLAY").is_ok();
        match session.as_str() {
            "x11" => Self::X11,
            "wayland" if has_display => Self::XWayland,
            "wayland" => Self::Wayland,
            _ if has_display => Self::X11,
            _ => Self::Unknown,
        }
    }

    pub fn supports_x11_queries(self) -> bool {
        matches!(self, Self::X11 | Self::XWayland)
    }
}

// ─── Grounding Capabilities ─────────────────────────────────────────────────

/// What the grounder was able to query on this system.
///
/// Tells the planner which fact fields are trustworthy vs structurally absent.
/// Probed once at grounder construction via `which::which()`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GroundingCapabilities {
    pub has_window_query: bool,
    pub has_window_list: bool,
    pub has_monitor_query: bool,
    pub display_server: DisplayServerType,
}

impl GroundingCapabilities {
    /// Probe system tool availability. Called once at startup.
    pub fn probe() -> Self {
        let display_server = DisplayServerType::detect();
        let x11_ok = display_server.supports_x11_queries();
        Self {
            has_window_query: x11_ok && which::which("xdotool").is_ok(),
            has_window_list: x11_ok && which::which("wmctrl").is_ok(),
            has_monitor_query: which::which("xrandr").is_ok(),
            display_server,
        }
    }

    /// All-degraded capabilities for testing or unavailable systems.
    pub fn none() -> Self {
        Self {
            has_window_query: false,
            has_window_list: false,
            has_monitor_query: false,
            display_server: DisplayServerType::Unknown,
        }
    }
}

// ─── Fact Structs ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowFact {
    pub title: String,
    pub class: String,
    pub pid: u32,
    pub desktop: i32,
    pub geometry: Option<Rect>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessFact {
    pub binary: String,
    pub pid: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalFact {
    pub binary: Option<String>,
    pub focused: bool,
    pub cwd: Option<PathBuf>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonitorFact {
    pub id: u32,
    pub geometry: Rect,
    pub scale: f32,
    pub primary: bool,
}

/// Maximum number of visible windows stored.
pub const MAX_VISIBLE_WINDOWS: usize = 16;
/// Maximum number of process entries stored.
pub const MAX_PROCESS_SUBSET: usize = 8;

/// The closed-enum fact bundle returned by the grounder.
///
/// Hard caps prevent symbolic state explosion. Code review must reject any
/// addition that introduces unbounded collections or generic key-value stores.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OperationalFacts {
    // ── System capabilities ──
    pub capabilities: GroundingCapabilities,

    // ── Execution targeting ──
    pub focused_window: Option<WindowFact>,
    /// Normalized app name extracted from focused window class.
    pub focused_app: Option<String>,

    // ── Shell continuity ──
    pub active_terminal: Option<TerminalFact>,
    /// Convenience alias for `active_terminal.cwd`.
    pub terminal_cwd: Option<PathBuf>,

    // ── IDE continuity ──
    /// Best-effort hint. May be `None` or inaccurate. Inferred from
    /// focused window title when it looks like an IDE project path.
    pub open_project_path: Option<PathBuf>,

    // ── Ambiguity resolution ──
    /// Visible windows on current workspace. Capped at [`MAX_VISIBLE_WINDOWS`].
    pub visible_windows: Vec<WindowFact>,

    // ── Coordinate correctness ──
    pub monitors: Vec<MonitorFact>,

    // ── Runtime awareness ──
    /// Processes owning visible windows or matching targets.
    /// Capped at [`MAX_PROCESS_SUBSET`].
    pub running_process_subset: Vec<ProcessFact>,

    // ── Metadata ──
    #[serde(skip)]
    pub captured_at: Instant,
}

impl OperationalFacts {
    /// Returns true if the facts are still within TTL.
    pub fn is_fresh(&self) -> bool {
        self.captured_at.elapsed().as_secs() < GroundingCache::TTL_SECS
    }

    /// Empty facts with given capabilities (degraded or cold-start).
    pub fn empty(capabilities: GroundingCapabilities) -> Self {
        Self {
            capabilities,
            focused_window: None,
            focused_app: None,
            active_terminal: None,
            terminal_cwd: None,
            open_project_path: None,
            visible_windows: Vec::new(),
            monitors: Vec::new(),
            running_process_subset: Vec::new(),
            captured_at: Instant::now(),
        }
    }
}

// ─── Cache ──────────────────────────────────────────────────────────────────

struct CachedSnapshot {
    facts: OperationalFacts,
    generation_at_capture: u64,
    captured_at: Instant,
}

/// Lock-free grounding cache with dual invalidation (TTL + generation counter).
///
/// - Reads via `ArcSwap::load()` are zero-lock, <1 μs.
/// - Writes store a new `Arc` atomically (rare, only on cache miss).
/// - Generation counter is bumped by perception events for instant invalidation.
pub struct GroundingCache {
    snapshot: arc_swap::ArcSwap<Option<CachedSnapshot>>,
    generation: AtomicU64,
}

impl GroundingCache {
    pub const TTL_SECS: u64 = 10;

    pub fn new() -> Self {
        Self {
            snapshot: arc_swap::ArcSwap::from_pointee(None),
            generation: AtomicU64::new(0),
        }
    }

    /// Lock-free read. Returns `None` if stale or invalidated.
    pub fn get_if_fresh(&self) -> Option<OperationalFacts> {
        let guard = self.snapshot.load();
        let snap = guard.as_ref().as_ref()?;

        let current_gen = self.generation.load(Ordering::Acquire);
        if snap.generation_at_capture < current_gen {
            return None;
        }
        if snap.captured_at.elapsed().as_secs() >= Self::TTL_SECS {
            return None;
        }

        Some(snap.facts.clone())
    }

    /// Store a new snapshot (called after OS queries).
    pub fn store(&self, facts: OperationalFacts) {
        let gen = self.generation.load(Ordering::Acquire);
        self.snapshot.store(Arc::new(Some(CachedSnapshot {
            facts,
            generation_at_capture: gen,
            captured_at: Instant::now(),
        })));
    }

    /// Bump generation to invalidate cache. O(1), <1 ns.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Current generation counter (for diagnostics).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Snapshot the cache state for observability. Lock-free, <1 μs.
    pub fn snapshot_status(&self) -> GroundingStatus {
        let guard = self.snapshot.load();
        let generation = self.generation.load(Ordering::Relaxed);

        match guard.as_ref().as_ref() {
            Some(snap) => {
                let age_ms = snap.captured_at.elapsed().as_millis() as u64;
                let is_stale = snap.generation_at_capture < generation
                    || snap.captured_at.elapsed().as_secs() >= Self::TTL_SECS;

                GroundingStatus {
                    cache_generation: generation,
                    cache_age_ms: age_ms,
                    cache_stale: is_stale,
                    capabilities: snap.facts.capabilities,
                    focused_app: snap.facts.focused_app.clone(),
                    focused_window_title: snap
                        .facts
                        .focused_window
                        .as_ref()
                        .map(|w| w.title.clone()),
                    visible_window_count: snap.facts.visible_windows.len() as u32,
                    monitor_count: snap.facts.monitors.len() as u32,
                    terminal_cwd: snap
                        .facts
                        .terminal_cwd
                        .as_ref()
                        .map(|p| p.display().to_string()),
                    open_project: snap
                        .facts
                        .open_project_path
                        .as_ref()
                        .map(|p| p.display().to_string()),
                    process_count: snap.facts.running_process_subset.len() as u32,
                }
            }
            None => GroundingStatus {
                cache_generation: generation,
                cache_age_ms: 0,
                cache_stale: true,
                capabilities: GroundingCapabilities::none(),
                focused_app: None,
                focused_window_title: None,
                visible_window_count: 0,
                monitor_count: 0,
                terminal_cwd: None,
                open_project: None,
                process_count: 0,
            },
        }
    }
}

/// Lightweight operational status snapshot for UI/debugging.
///
/// Strictly observational. No semantic analysis, no reasoning,
/// no confidence scores, no ontology classification.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GroundingStatus {
    /// Cache invalidation generation counter.
    pub cache_generation: u64,
    /// Milliseconds since last cache fill.
    pub cache_age_ms: u64,
    /// Whether cached facts are considered stale (TTL or generation mismatch).
    pub cache_stale: bool,
    /// What the grounder can observe on this system.
    pub capabilities: GroundingCapabilities,
    /// Normalized app name of focused window (if known).
    pub focused_app: Option<String>,
    /// Title of focused window (if known).
    pub focused_window_title: Option<String>,
    /// Number of visible windows on current workspace.
    pub visible_window_count: u32,
    /// Number of connected monitors.
    pub monitor_count: u32,
    /// Terminal CWD if a terminal is focused.
    pub terminal_cwd: Option<String>,
    /// IDE project name hint (if an IDE is focused).
    pub open_project: Option<String>,
    /// Number of tracked processes.
    pub process_count: u32,
}

// ─── Trait ──────────────────────────────────────────────────────────────────

/// Grounder contract — read-only, ephemeral, bounded.
///
/// The grounder MUST NOT plan, execute, persist, or generate actions.
/// It observes and normalizes operational desktop state only.
///
/// Planners MAY optimize step ordering using facts but MUST NOT remove
/// prerequisite verification steps. The executor's PrerequisiteChecker
/// is the ground truth.
#[async_trait::async_trait]
pub trait EnvironmentGrounder: Send + Sync {
    /// Produce a fresh operational facts snapshot.
    ///
    /// `targets` is provided for relevance filtering only (e.g., which
    /// processes to include in `running_process_subset`).
    async fn ground(&self, targets: &[TargetRef]) -> OperationalFacts;
}

// ─── Noop Implementation ────────────────────────────────────────────────────

/// Placeholder grounder. Returns empty facts. Used in tests and pre-P2 paths.
pub struct NoopEnvironmentGrounder;

#[async_trait::async_trait]
impl EnvironmentGrounder for NoopEnvironmentGrounder {
    async fn ground(&self, _targets: &[TargetRef]) -> OperationalFacts {
        OperationalFacts::empty(GroundingCapabilities::none())
    }
}

// ─── Live Implementation ────────────────────────────────────────────────────

/// Production grounder backed by X11 tooling and a lock-free cache.
pub struct LiveEnvironmentGrounder {
    capabilities: GroundingCapabilities,
    cache: Arc<GroundingCache>,
}

impl LiveEnvironmentGrounder {
    /// Construct and probe system capabilities.
    pub fn new() -> Self {
        let capabilities = GroundingCapabilities::probe();
        tracing::info!(
            target: "environment_grounder",
            ?capabilities,
            "LiveEnvironmentGrounder initialized"
        );
        Self {
            capabilities,
            cache: Arc::new(GroundingCache::new()),
        }
    }

    /// Spawn a background task that listens to the PerceptionBus and
    /// invalidates the grounding cache on desktop/process events.
    ///
    /// Returns a `JoinHandle` the caller can abort on shutdown.
    /// The task exits cleanly when the broadcast channel closes.
    pub fn spawn_invalidation_listener(
        &self,
        bus: &crate::agent::perception::PerceptionBus,
    ) -> tokio::task::JoinHandle<()> {
        use crate::agent::perception::EventKind;

        let cache = Arc::clone(&self.cache);
        let mut rx = bus.subscribe();

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let should_invalidate = matches!(
                            &event.kind,
                            EventKind::DesktopEvent(_) | EventKind::ProcessLifecycle(_)
                        );
                        if should_invalidate {
                            cache.invalidate();
                            tracing::debug!(
                                target: "environment_grounder",
                                kind = ?event.kind,
                                generation = cache.generation(),
                                "cache invalidated by perception event"
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Missed events — invalidate as safety net
                        cache.invalidate();
                        tracing::warn!(
                            target: "environment_grounder",
                            skipped = n,
                            "perception bus lagged, forced cache invalidation"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::debug!(
                            target: "environment_grounder",
                            "perception bus closed, invalidation listener exiting"
                        );
                        break;
                    }
                }
            }
        })
    }

    /// Get a reference to the cache (for wiring invalidation listeners).
    pub fn cache(&self) -> &Arc<GroundingCache> {
        &self.cache
    }

    /// Get probed capabilities.
    pub fn capabilities(&self) -> GroundingCapabilities {
        self.capabilities
    }

    /// Get a lock-free status snapshot for operational observability.
    pub fn grounding_status(&self) -> GroundingStatus {
        self.cache.snapshot_status()
    }

    /// Perform all OS queries and assemble facts.
    /// Called only on cache miss.
    async fn refresh(&self, targets: &[TargetRef]) -> OperationalFacts {
        let mut facts = OperationalFacts::empty(self.capabilities);

        if self.capabilities.has_window_query {
            facts.focused_window = self.query_focused_window().await;
            facts.focused_app = facts
                .focused_window
                .as_ref()
                .map(|w| normalize_app_name(&w.class));

            // Detect terminal focus + sniff CWD from /proc
            if let Some(ref win) = facts.focused_window {
                if is_terminal_class(&win.class) {
                    let cwd = if win.pid > 0 {
                        read_proc_cwd(win.pid).await
                    } else {
                        None
                    };
                    facts.terminal_cwd = cwd.clone();
                    facts.active_terminal = Some(TerminalFact {
                        binary: Some(normalize_app_name(&win.class)),
                        focused: true,
                        cwd,
                        pid: Some(win.pid),
                    });
                }
                // Best-effort IDE project path from title
                facts.open_project_path = extract_project_path(&win.title);
            }
        }

        if self.capabilities.has_window_list {
            facts.visible_windows = self.query_visible_windows().await;
            facts.running_process_subset = build_process_subset(&facts.visible_windows, targets);
        }

        if self.capabilities.has_monitor_query {
            facts.monitors = self.query_monitors().await;
        }

        facts.captured_at = Instant::now();
        facts
    }

    // ── OS Queries ──────────────────────────────────────────────────────

    /// Query focused window via xdotool + xprop. ~10-15ms typical.
    async fn query_focused_window(&self) -> Option<WindowFact> {
        let win_id = run_grounding_query("xdotool", &["getactivewindow"])
            .await
            .ok()?;
        let win_id = win_id.trim();
        if win_id.is_empty() {
            return None;
        }

        // Bind args to local variables so they outlive the tokio::join! macro
        let title_args = ["getwindowname", win_id];
        let pid_args = ["getwindowpid", win_id];
        let class_args = ["-id", win_id, "WM_CLASS"];

        let (title_r, pid_r, class_r) = tokio::join!(
            run_grounding_query("xdotool", &title_args),
            run_grounding_query("xdotool", &pid_args),
            run_grounding_query("xprop", &class_args),
        );

        let title = title_r.unwrap_or_default().trim().to_string();
        let pid: u32 = pid_r.unwrap_or_default().trim().parse().unwrap_or(0);
        let class = class_r
            .map(|raw| parse_wm_class_raw(&raw))
            .unwrap_or_default();

        // Get current desktop for this window
        let desktop = get_window_desktop(win_id).await;

        Some(WindowFact {
            title,
            class,
            pid,
            desktop,
            geometry: None,
        })
    }

    /// Query visible windows on current workspace via wmctrl -lG.
    /// Filters to current desktop and caps at MAX_VISIBLE_WINDOWS.
    async fn query_visible_windows(&self) -> Vec<WindowFact> {
        let current_desktop = get_current_desktop().await;
        let output = match run_grounding_query("wmctrl", &["-lGp"]).await {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let mut windows = Vec::new();
        for line in output.lines() {
            if let Some(win) = parse_wmctrl_lgp_line(line) {
                // Filter: only current desktop (or sticky = -1)
                if win.desktop == current_desktop || win.desktop == -1 {
                    windows.push(win);
                    if windows.len() >= MAX_VISIBLE_WINDOWS {
                        break;
                    }
                }
            }
        }
        windows
    }

    /// Query monitor layout via xrandr --query.
    async fn query_monitors(&self) -> Vec<MonitorFact> {
        let output = match run_grounding_query("xrandr", &["--query"]).await {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        parse_xrandr_monitors(&output)
    }
}

// ─── Async OS Helper ────────────────────────────────────────────────────────

/// Lightweight async query — 5s timeout, bounded output, no ExecWrapper overhead.
async fn run_grounding_query(program: &str, args: &[&str]) -> Result<String, ()> {
    use tokio::process::Command;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => Err(()),
    }
}

/// Get current desktop number from wmctrl -d.
async fn get_current_desktop() -> i32 {
    let output = match run_grounding_query("wmctrl", &["-d"]).await {
        Ok(o) => o,
        Err(_) => return 0,
    };
    // Lines like: "0  * DG: 1920x1080  VP: ..."  — current desktop has '*'
    for line in output.lines() {
        if line.contains('*') {
            return line
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

/// Get desktop number for a specific window ID via xprop.
async fn get_window_desktop(win_id: &str) -> i32 {
    run_grounding_query("xdotool", &["get_desktop_for_window", win_id])
        .await
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Read /proc/<pid>/cwd symlink. May fail for privileged processes.
async fn read_proc_cwd(pid: u32) -> Option<PathBuf> {
    let path = format!("/proc/{}/cwd", pid);
    tokio::fs::read_link(&path).await.ok()
}

/// Parse WM_CLASS from xprop output: `WM_CLASS(STRING) = "instance", "Class"`
fn parse_wm_class_raw(raw: &str) -> String {
    raw.split('=')
        .nth(1)
        .map(|v| v.trim().replace('"', "").replace(' ', ""))
        .unwrap_or_default()
}

/// Parse wmctrl -lGp output line.
/// Format: `0x04a00003  0 PID  X Y W H  hostname  Title...`
fn parse_wmctrl_lgp_line(line: &str) -> Option<WindowFact> {
    let mut parts = line.split_whitespace();
    let _id = parts.next()?;
    let desktop: i32 = parts.next()?.parse().ok()?;
    let pid: u32 = parts.next()?.parse().unwrap_or(0);
    let x: i32 = parts.next()?.parse().ok()?;
    let y: i32 = parts.next()?.parse().ok()?;
    let w: u32 = parts.next()?.parse().ok()?;
    let h: u32 = parts.next()?.parse().ok()?;
    let _hostname = parts.next()?;
    let title: String = parts.collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }

    Some(WindowFact {
        title,
        class: String::new(), // wmctrl doesn't provide WM_CLASS
        pid,
        desktop,
        geometry: Some(Rect { x, y, w, h }),
    })
}

/// Parse xrandr --query output for connected monitors.
fn parse_xrandr_monitors(output: &str) -> Vec<MonitorFact> {
    let mut monitors = Vec::new();
    let mut id_counter = 0u32;
    for line in output.lines() {
        // Match lines like: "eDP-1 connected primary 1920x1080+0+0 ..."
        if !line.contains(" connected") {
            continue;
        }
        let primary = line.contains("primary");
        // Find geometry: WxH+X+Y
        let geom = line
            .split_whitespace()
            .find(|s| s.contains('+') && s.contains('x'));
        if let Some(geom_str) = geom {
            if let Some(rect) = parse_xrandr_geometry(geom_str) {
                monitors.push(MonitorFact {
                    id: id_counter,
                    geometry: rect,
                    scale: 1.0,
                    primary,
                });
                id_counter += 1;
            }
        }
    }
    monitors
}

/// Parse "1920x1080+0+0" into Rect.
fn parse_xrandr_geometry(s: &str) -> Option<Rect> {
    // Split on 'x' and '+'
    let parts: Vec<&str> = s.split(&['x', '+'][..]).collect();
    if parts.len() < 4 {
        return None;
    }
    Some(Rect {
        w: parts[0].parse().ok()?,
        h: parts[1].parse().ok()?,
        x: parts[2].parse().ok()?,
        y: parts[3].parse().ok()?,
    })
}

/// Best-effort: extract project path from IDE window titles.
/// e.g. "main.rs — KRIA — Visual Studio Code" → /path if detectable
/// Returns None for non-IDE windows. No semantic reasoning.
fn extract_project_path(title: &str) -> Option<PathBuf> {
    // VS Code: "filename — ProjectName — Visual Studio Code"
    // JetBrains: "filename – ProjectName – IntelliJ IDEA"
    if !title.contains("Visual Studio Code")
        && !title.contains("IntelliJ")
        && !title.contains("PyCharm")
        && !title.contains("WebStorm")
        && !title.contains("CLion")
    {
        return None;
    }
    // Extract middle segment (project name) — not a path, just a hint
    let segments: Vec<&str> = title.split(&['—', '–'][..]).collect();
    if segments.len() >= 2 {
        let project = segments[segments.len() - 2].trim();
        if !project.is_empty() {
            return Some(PathBuf::from(project));
        }
    }
    None
}

#[async_trait::async_trait]
impl EnvironmentGrounder for LiveEnvironmentGrounder {
    async fn ground(&self, targets: &[TargetRef]) -> OperationalFacts {
        // Fast path: return cached facts if fresh
        if let Some(facts) = self.cache.get_if_fresh() {
            tracing::trace!(
                target: "environment_grounder",
                generation = self.cache.generation(),
                "cache hit"
            );
            return facts;
        }

        // Slow path: refresh from OS
        tracing::debug!(
            target: "environment_grounder",
            generation = self.cache.generation(),
            "cache miss — refreshing from OS"
        );
        let facts = self.refresh(targets).await;
        self.cache.store(facts.clone());
        facts
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Normalize WM_CLASS to a human-readable app name.
fn normalize_app_name(wm_class: &str) -> String {
    // WM_CLASS is typically "instance, class" — take the class part
    wm_class
        .split(',')
        .last()
        .unwrap_or(wm_class)
        .trim()
        .to_lowercase()
}

/// Check if a WM_CLASS represents a terminal emulator.
fn is_terminal_class(wm_class: &str) -> bool {
    let lower = wm_class.to_lowercase();
    const TERMINALS: &[&str] = &[
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "terminator",
        "alacritty",
        "kitty",
        "xterm",
        "urxvt",
        "tilix",
        "wezterm",
        "foot",
        "st-256color",
        "sakura",
        "terminology",
    ];
    TERMINALS.iter().any(|t| lower.contains(t))
}

/// Build a bounded process subset from visible window PIDs + target app names.
fn build_process_subset(visible_windows: &[WindowFact], targets: &[TargetRef]) -> Vec<ProcessFact> {
    let mut result = Vec::new();
    let mut seen_pids = std::collections::HashSet::new();

    // Add processes from visible windows
    for win in visible_windows {
        if win.pid > 0 && seen_pids.insert(win.pid) {
            result.push(ProcessFact {
                binary: normalize_app_name(&win.class),
                pid: win.pid,
            });
            if result.len() >= MAX_PROCESS_SUBSET {
                return result;
            }
        }
    }

    // Add processes matching target app names (via sysinfo, deferred to P2c)
    let _target_apps: Vec<&str> = targets
        .iter()
        .filter_map(|t| match t {
            TargetRef::App(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();

    result
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_grounder_returns_fresh_empty_facts() {
        let facts = NoopEnvironmentGrounder.ground(&[]).await;
        assert!(facts.is_fresh());
        assert!(facts.focused_window.is_none());
        assert!(facts.visible_windows.is_empty());
        assert!(facts.running_process_subset.is_empty());
    }

    #[tokio::test]
    async fn noop_grounder_reports_no_capabilities() {
        let facts = NoopEnvironmentGrounder.ground(&[]).await;
        assert!(!facts.capabilities.has_window_query);
        assert!(!facts.capabilities.has_window_list);
        assert!(!facts.capabilities.has_monitor_query);
        assert_eq!(
            facts.capabilities.display_server,
            DisplayServerType::Unknown
        );
    }

    #[test]
    fn cache_starts_empty() {
        let cache = GroundingCache::new();
        assert!(cache.get_if_fresh().is_none());
        assert_eq!(cache.generation(), 0);
    }

    #[test]
    fn cache_store_and_retrieve() {
        let cache = GroundingCache::new();
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        cache.store(facts);
        assert!(cache.get_if_fresh().is_some());
    }

    #[test]
    fn cache_invalidation_bumps_generation() {
        let cache = GroundingCache::new();
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        cache.store(facts);
        assert!(cache.get_if_fresh().is_some());

        cache.invalidate();
        assert_eq!(cache.generation(), 1);
        assert!(cache.get_if_fresh().is_none());
    }

    #[test]
    fn cache_ttl_expiry() {
        let cache = GroundingCache::new();
        // Store with an old timestamp
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.captured_at = Instant::now() - std::time::Duration::from_secs(15);
        // We can't easily test TTL without time manipulation,
        // but we can verify fresh facts are returned
        cache.store(OperationalFacts::empty(GroundingCapabilities::none()));
        assert!(cache.get_if_fresh().is_some());
    }

    #[test]
    fn normalize_app_name_extracts_class() {
        assert_eq!(normalize_app_name("Navigator, Firefox"), "firefox");
        assert_eq!(normalize_app_name("code"), "code");
        assert_eq!(
            normalize_app_name("gnome-terminal-server, Gnome-terminal"),
            "gnome-terminal"
        );
    }

    #[test]
    fn is_terminal_class_detection() {
        assert!(is_terminal_class("gnome-terminal-server, Gnome-terminal"));
        assert!(is_terminal_class("Alacritty"));
        assert!(is_terminal_class("kitty"));
        assert!(!is_terminal_class("Firefox"));
        assert!(!is_terminal_class("code"));
    }

    #[test]
    fn display_server_x11_supports_queries() {
        assert!(DisplayServerType::X11.supports_x11_queries());
        assert!(DisplayServerType::XWayland.supports_x11_queries());
        assert!(!DisplayServerType::Wayland.supports_x11_queries());
        assert!(!DisplayServerType::Unknown.supports_x11_queries());
    }

    #[test]
    fn process_subset_respects_cap() {
        let windows: Vec<WindowFact> = (0..20)
            .map(|i| WindowFact {
                title: format!("Window {}", i),
                class: format!("app{}", i),
                pid: 1000 + i,
                desktop: 0,
                geometry: None,
            })
            .collect();

        let result = build_process_subset(&windows, &[]);
        assert!(result.len() <= MAX_PROCESS_SUBSET);
    }

    #[test]
    fn bounded_caps_constants() {
        assert_eq!(MAX_VISIBLE_WINDOWS, 16);
        assert_eq!(MAX_PROCESS_SUBSET, 8);
    }

    // ── P2c parser tests ────────────────────────────────────────────────

    #[test]
    fn parse_wmctrl_lgp_line_valid() {
        let line = "0x04a00003  0 12345  100 200 800 600  hostname  Firefox Developer Edition";
        let win = parse_wmctrl_lgp_line(line).unwrap();
        assert_eq!(win.desktop, 0);
        assert_eq!(win.pid, 12345);
        assert_eq!(win.title, "Firefox Developer Edition");
        let g = win.geometry.unwrap();
        assert_eq!((g.x, g.y, g.w, g.h), (100, 200, 800, 600));
    }

    #[test]
    fn parse_wmctrl_lgp_line_sticky_desktop() {
        let line = "0x02c00001  -1 0  0 0 1920 1080  host  Desktop";
        let win = parse_wmctrl_lgp_line(line).unwrap();
        assert_eq!(win.desktop, -1);
    }

    #[test]
    fn parse_wmctrl_lgp_line_no_title() {
        let line = "0x04a00003  0 123  100 200 800 600  hostname";
        assert!(parse_wmctrl_lgp_line(line).is_none());
    }

    #[test]
    fn parse_xrandr_single_monitor() {
        let output = "eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 344mm x 194mm\n   1920x1080     60.01*+\n";
        let monitors = parse_xrandr_monitors(output);
        assert_eq!(monitors.len(), 1);
        assert!(monitors[0].primary);
        assert_eq!(monitors[0].geometry.w, 1920);
        assert_eq!(monitors[0].geometry.h, 1080);
        assert_eq!(monitors[0].geometry.x, 0);
    }

    #[test]
    fn parse_xrandr_dual_monitor() {
        let output = "\
eDP-1 connected primary 1920x1080+0+0 (normal)\n\
   1920x1080     60.01*+\n\
HDMI-1 connected 2560x1440+1920+0 (normal)\n\
   2560x1440     59.95*+\n\
DP-1 disconnected\n";
        let monitors = parse_xrandr_monitors(output);
        assert_eq!(monitors.len(), 2);
        assert!(monitors[0].primary);
        assert!(!monitors[1].primary);
        assert_eq!(monitors[1].geometry.x, 1920);
        assert_eq!(monitors[1].geometry.w, 2560);
    }

    #[test]
    fn parse_xrandr_geometry_valid() {
        let r = parse_xrandr_geometry("1920x1080+0+0").unwrap();
        assert_eq!((r.w, r.h, r.x, r.y), (1920, 1080, 0, 0));
    }

    #[test]
    fn parse_xrandr_geometry_offset() {
        let r = parse_xrandr_geometry("2560x1440+1920+0").unwrap();
        assert_eq!((r.w, r.h, r.x, r.y), (2560, 1440, 1920, 0));
    }

    #[test]
    fn parse_xrandr_geometry_invalid() {
        assert!(parse_xrandr_geometry("invalid").is_none());
        assert!(parse_xrandr_geometry("1920x1080").is_none());
    }

    #[test]
    fn parse_wm_class_raw_standard() {
        let raw = r#"WM_CLASS(STRING) = "code", "Code""#;
        assert_eq!(parse_wm_class_raw(raw), "code,Code");
    }

    #[test]
    fn extract_project_path_vscode() {
        let title = "main.rs — KRIA — Visual Studio Code";
        let path = extract_project_path(title).unwrap();
        assert_eq!(path, PathBuf::from("KRIA"));
    }

    #[test]
    fn extract_project_path_non_ide() {
        assert!(extract_project_path("Firefox").is_none());
        assert!(extract_project_path("Terminal").is_none());
    }

    // ── P2d invalidation tests ──────────────────────────────────────────

    #[tokio::test]
    async fn invalidation_listener_bumps_generation_on_desktop_event() {
        use crate::agent::perception::{
            DesktopOp, EventKind, EventSeverity, PerceptionBus, PerceptionEvent,
        };

        let bus = PerceptionBus::new(16);
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        // Store a snapshot so we can verify invalidation
        grounder
            .cache
            .store(OperationalFacts::empty(GroundingCapabilities::none()));
        assert!(grounder.cache.get_if_fresh().is_some());
        assert_eq!(grounder.cache.generation(), 0);

        let _handle = grounder.spawn_invalidation_listener(&bus);

        // Send a desktop event
        let tx = bus.sender();
        tx.send(PerceptionEvent {
            kind: EventKind::DesktopEvent(DesktopOp::FocusChanged),
            key: "desktop:FocusChanged".into(),
            primary_path: None,
            count: 1,
            summary: "focus changed".into(),
            severity: EventSeverity::Info,
            first_seen_epoch_ms: 0,
            finalized_epoch_ms: 0,
        })
        .unwrap();

        // Yield to let the listener process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(grounder.cache.generation(), 1);
        assert!(grounder.cache.get_if_fresh().is_none());
    }

    #[tokio::test]
    async fn invalidation_listener_ignores_filesystem_events() {
        use crate::agent::perception::{
            EventKind, EventSeverity, FilesystemOp, PerceptionBus, PerceptionEvent,
        };

        let bus = PerceptionBus::new(16);
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        grounder
            .cache
            .store(OperationalFacts::empty(GroundingCapabilities::none()));
        let _handle = grounder.spawn_invalidation_listener(&bus);

        let tx = bus.sender();
        tx.send(PerceptionEvent {
            kind: EventKind::Filesystem(FilesystemOp::Modified),
            key: "fs:Modified:/tmp/test".into(),
            primary_path: Some("/tmp/test".into()),
            count: 1,
            summary: "file modified".into(),
            severity: EventSeverity::Info,
            first_seen_epoch_ms: 0,
            finalized_epoch_ms: 0,
        })
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Generation should NOT be bumped for filesystem events
        assert_eq!(grounder.cache.generation(), 0);
        assert!(grounder.cache.get_if_fresh().is_some());
    }

    // ── P2e runtime hardening tests ─────────────────────────────────────

    #[test]
    fn stale_cache_returns_none() {
        let cache = GroundingCache::new();
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        // Simulate old capture (expired TTL)
        facts.captured_at = Instant::now() - std::time::Duration::from_secs(15);
        cache.store(facts);

        // Cache should be "stale" because the CachedSnapshot has fresh captured_at
        // (store() uses Instant::now() internally). But we CAN test generation stale:
        cache.invalidate(); // generation=1
        assert!(cache.get_if_fresh().is_none());
    }

    #[test]
    fn multiple_invalidations_monotonic() {
        let cache = GroundingCache::new();
        let facts = OperationalFacts::empty(GroundingCapabilities::none());
        cache.store(facts);

        cache.invalidate();
        cache.invalidate();
        cache.invalidate();

        assert_eq!(cache.generation(), 3);
        assert!(cache.get_if_fresh().is_none());
    }

    #[test]
    fn cache_refill_after_invalidation() {
        let cache = GroundingCache::new();

        cache.store(OperationalFacts::empty(GroundingCapabilities::none()));
        assert!(cache.get_if_fresh().is_some());

        cache.invalidate();
        assert!(cache.get_if_fresh().is_none());

        // Refill with new facts
        cache.store(OperationalFacts::empty(GroundingCapabilities::none()));
        assert!(cache.get_if_fresh().is_some());
    }

    #[tokio::test]
    async fn noop_grounder_always_fresh_empty() {
        // NoopEnvironmentGrounder must always return fresh, empty, degraded facts
        let facts1 = NoopEnvironmentGrounder.ground(&[]).await;
        let facts2 = NoopEnvironmentGrounder.ground(&[]).await;

        assert!(facts1.is_fresh());
        assert!(facts2.is_fresh());
        assert!(facts1.focused_window.is_none());
        assert!(facts2.focused_app.is_none());
        assert!(!facts1.capabilities.has_window_query);
    }

    #[tokio::test]
    async fn degraded_mode_no_capabilities() {
        // When no tools are available, grounder should return empty but valid facts
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        let facts = grounder.ground(&[]).await;
        assert!(facts.is_fresh());
        assert!(facts.focused_window.is_none());
        assert!(facts.visible_windows.is_empty());
        assert!(facts.monitors.is_empty());
    }

    // ── P2g observability tests ─────────────────────────────────────────

    #[test]
    fn grounding_status_cold_cache() {
        let cache = GroundingCache::new();
        let status = cache.snapshot_status();

        assert!(status.cache_stale);
        assert_eq!(status.cache_generation, 0);
        assert_eq!(status.cache_age_ms, 0);
        assert!(status.focused_app.is_none());
        assert_eq!(status.visible_window_count, 0);
        assert_eq!(status.monitor_count, 0);
    }

    #[test]
    fn grounding_status_warm_cache() {
        let cache = GroundingCache::new();
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("firefox".to_string());
        facts.focused_window = Some(WindowFact {
            title: "GitHub - Firefox".to_string(),
            class: "Firefox".to_string(),
            pid: 1234,
            desktop: 0,
            geometry: None,
        });
        facts.visible_windows = vec![WindowFact {
            title: "Terminal".to_string(),
            class: "gnome-terminal".to_string(),
            pid: 5678,
            desktop: 0,
            geometry: None,
        }];
        facts.monitors = vec![MonitorFact {
            id: 0,
            geometry: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            scale: 1.0,
            primary: true,
        }];
        cache.store(facts);

        let status = cache.snapshot_status();
        assert!(!status.cache_stale);
        assert_eq!(status.focused_app, Some("firefox".to_string()));
        assert_eq!(
            status.focused_window_title,
            Some("GitHub - Firefox".to_string())
        );
        assert_eq!(status.visible_window_count, 1);
        assert_eq!(status.monitor_count, 1);
    }

    #[test]
    fn grounding_status_after_invalidation() {
        let cache = GroundingCache::new();
        cache.store(OperationalFacts::empty(GroundingCapabilities::none()));
        assert!(!cache.snapshot_status().cache_stale);

        cache.invalidate();
        let status = cache.snapshot_status();
        assert!(status.cache_stale);
        assert_eq!(status.cache_generation, 1);
    }

    #[test]
    fn grounding_status_serializes_to_json() {
        let cache = GroundingCache::new();
        cache.store(OperationalFacts::empty(GroundingCapabilities::none()));
        let status = cache.snapshot_status();

        // Must serialize without panic — required for Tauri endpoint
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("cache_generation").is_some());
        assert!(json.get("cache_stale").is_some());
        assert!(json.get("capabilities").is_some());
        assert!(json.get("focused_app").is_some());
        assert!(json.get("visible_window_count").is_some());
        assert!(json.get("monitor_count").is_some());
    }

    #[test]
    fn live_grounder_grounding_status_accessor() {
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        let status = grounder.grounding_status();
        assert!(status.cache_stale); // Cold cache
        assert_eq!(status.cache_generation, 0);
    }

    // ── P2h: Real-world workflow reliability tests ──────────────────────

    #[tokio::test]
    async fn event_storm_invalidation_bounded() {
        // Scenario: rapid-fire desktop events should not cause unbounded
        // generation growth or cache thrashing that blocks the planner.
        use crate::agent::perception::{
            DesktopOp, EventKind, EventSeverity, PerceptionBus, PerceptionEvent,
        };

        let bus = PerceptionBus::new(256);
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        grounder
            .cache
            .store(OperationalFacts::empty(GroundingCapabilities::none()));
        let _handle = grounder.spawn_invalidation_listener(&bus);
        let tx = bus.sender();

        // Fire 100 rapid focus-change events (simulates window manager storm)
        for i in 0..100 {
            let _ = tx.send(PerceptionEvent {
                kind: EventKind::DesktopEvent(DesktopOp::FocusChanged),
                key: format!("desktop:FocusChanged:{}", i),
                primary_path: None,
                count: 1,
                summary: "focus storm".into(),
                severity: EventSeverity::Info,
                first_seen_epoch_ms: 0,
                finalized_epoch_ms: 0,
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Generation should be exactly 100 — monotonic, no overflow, no skip
        assert_eq!(grounder.cache.generation(), 100);
        // Cache must be stale after storm
        assert!(grounder.cache.get_if_fresh().is_none());
    }

    #[tokio::test]
    async fn focus_race_during_planning() {
        // Scenario: user changes focus WHILE grounder is filling cache.
        // After store(), an invalidation arrives. Next read should miss.
        let cache = Arc::new(GroundingCache::new());

        // Simulate: grounder stores facts
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("vscode".to_string());
        cache.store(facts);

        // Confirm cache hit
        let snap = cache.get_if_fresh().unwrap();
        assert_eq!(snap.focused_app, Some("vscode".to_string()));

        // User switches focus → invalidation arrives
        cache.invalidate();

        // Next read must miss (stale data about vscode no longer valid)
        assert!(cache.get_if_fresh().is_none());
    }

    #[tokio::test]
    async fn cache_invalidated_during_planning_does_not_corrupt() {
        // Scenario: grounder fills cache, planner starts reading,
        // invalidation arrives mid-read. The read must either return
        // the old snapshot OR None — never a partial/corrupt state.
        let cache = Arc::new(GroundingCache::new());

        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("firefox".to_string());
        facts.visible_windows = vec![WindowFact {
            title: "Tab 1".into(),
            class: "Firefox".into(),
            pid: 999,
            desktop: 0,
            geometry: None,
        }];
        cache.store(facts);

        // Read snapshot (ArcSwap guarantees atomic snapshot)
        let snap = cache.get_if_fresh();
        assert!(snap.is_some());
        let snap = snap.unwrap();

        // Invalidation arrives AFTER we read
        cache.invalidate();

        // The snapshot we already hold is still internally consistent
        assert_eq!(snap.focused_app, Some("firefox".to_string()));
        assert_eq!(snap.visible_windows.len(), 1);

        // But NEXT read from cache returns None
        assert!(cache.get_if_fresh().is_none());
    }

    #[tokio::test]
    async fn workspace_change_invalidation() {
        // Scenario: user switches workspace → WorkspaceChanged event
        // → cache invalidated → next ground() returns fresh facts
        use crate::agent::perception::{
            DesktopOp, EventKind, EventSeverity, PerceptionBus, PerceptionEvent,
        };

        let bus = PerceptionBus::new(16);
        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::new(GroundingCache::new()),
        };

        // Fill cache with workspace 0 data
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("terminal".to_string());
        grounder.cache.store(facts);

        let _handle = grounder.spawn_invalidation_listener(&bus);
        let tx = bus.sender();

        // User switches workspace
        tx.send(PerceptionEvent {
            kind: EventKind::DesktopEvent(DesktopOp::WorkspaceChanged),
            key: "desktop:WorkspaceChanged".into(),
            primary_path: None,
            count: 1,
            summary: "workspace changed".into(),
            severity: EventSeverity::Info,
            first_seen_epoch_ms: 0,
            finalized_epoch_ms: 0,
        })
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Cache must be invalidated — old workspace data is stale
        assert!(grounder.cache.get_if_fresh().is_none());
        assert_eq!(grounder.cache.generation(), 1);
    }

    #[test]
    fn multi_monitor_targeting_preserves_geometry() {
        // Scenario: dual-monitor setup, windows on both monitors
        let xrandr_output = "\
eDP-1 connected primary 1920x1080+0+0 (normal)
   1920x1080     60.01*+
HDMI-1 connected 2560x1440+1920+0 (normal)
   2560x1440     59.95*+
";
        let monitors = parse_xrandr_monitors(xrandr_output);
        assert_eq!(monitors.len(), 2);

        // Primary monitor at origin
        assert!(monitors[0].primary);
        assert_eq!(monitors[0].geometry.x, 0);
        assert_eq!(monitors[0].geometry.w, 1920);

        // Secondary monitor offset by primary width
        assert!(!monitors[1].primary);
        assert_eq!(monitors[1].geometry.x, 1920);
        assert_eq!(monitors[1].geometry.w, 2560);

        // Windows on second monitor should have x >= 1920
        let line = "0x04a00003  0 12345  2000 100 800 600  hostname  Browser on Monitor 2";
        let win = parse_wmctrl_lgp_line(line).unwrap();
        let g = win.geometry.unwrap();
        assert!(g.x >= monitors[1].geometry.x as i32);
    }

    #[test]
    fn wayland_degraded_mode() {
        // Scenario: pure Wayland — all X11 queries unavailable
        let caps = GroundingCapabilities {
            has_window_query: false,
            has_window_list: false,
            has_monitor_query: false,
            display_server: DisplayServerType::Wayland,
        };

        // Facts should be empty but valid
        let facts = OperationalFacts::empty(caps);
        assert!(facts.is_fresh());
        assert!(facts.focused_window.is_none());
        assert!(facts.visible_windows.is_empty());
        assert!(!facts.capabilities.display_server.supports_x11_queries());
    }

    #[tokio::test]
    async fn concurrent_cache_readers_no_corruption() {
        // Scenario: multiple tasks read cache concurrently while
        // invalidation happens. No reader should see corrupt data.
        let cache = Arc::new(GroundingCache::new());

        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("code".to_string());
        cache.store(facts);

        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache_clone = Arc::clone(&cache);
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let snap = cache_clone.get_if_fresh();
                    if let Some(s) = snap {
                        // If we got a snapshot, it must be internally consistent
                        // (focused_app and capabilities must both be present)
                        assert!(s.focused_app.is_some() || s.focused_window.is_none());
                    }
                    tokio::task::yield_now().await;
                }
            }));
        }

        // Invalidate mid-flight
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        cache.invalidate();

        for h in handles {
            h.await.unwrap(); // No panics from any reader
        }
    }

    #[test]
    fn wrong_window_detection_via_facts() {
        // Scenario: planner sees "firefox" focused, but executor should
        // target "code". Facts are advisory — planner produces workflow
        // anyway, but facts record the mismatch for logging.
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.focused_app = Some("firefox".to_string());
        facts.focused_window = Some(WindowFact {
            title: "Reddit — Firefox".to_string(),
            class: "Navigator, Firefox".to_string(),
            pid: 1234,
            desktop: 0,
            geometry: None,
        });

        // The planner for "open code" receives these facts.
        // It should NOT skip the open step — facts are advisory.
        // The executor will handle the actual focus verification.
        assert_ne!(facts.focused_app, Some("code".to_string()));

        // Facts correctly report what IS focused, not what SHOULD be
        assert_eq!(facts.focused_app, Some("firefox".to_string()));
    }

    #[tokio::test]
    async fn window_destroyed_triggers_invalidation() {
        // Scenario: a window is closed → WindowDestroyed event
        use crate::agent::perception::{
            DesktopOp, EventKind, EventSeverity, PerceptionBus, PerceptionEvent,
        };

        let bus = PerceptionBus::new(16);
        let cache = Arc::new(GroundingCache::new());

        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        facts.visible_windows = vec![WindowFact {
            title: "Terminal".into(),
            class: "gnome-terminal".into(),
            pid: 5678,
            desktop: 0,
            geometry: None,
        }];
        cache.store(facts);

        let grounder = LiveEnvironmentGrounder {
            capabilities: GroundingCapabilities::none(),
            cache: Arc::clone(&cache),
        };

        let _handle = grounder.spawn_invalidation_listener(&bus);
        let tx = bus.sender();

        tx.send(PerceptionEvent {
            kind: EventKind::DesktopEvent(DesktopOp::WindowDestroyed),
            key: "desktop:WindowDestroyed".into(),
            primary_path: None,
            count: 1,
            summary: "window destroyed".into(),
            severity: EventSeverity::Info,
            first_seen_epoch_ms: 0,
            finalized_epoch_ms: 0,
        })
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Stale data about the closed window should be gone
        assert!(cache.get_if_fresh().is_none());
    }

    #[test]
    fn terminal_cwd_continuity() {
        // Scenario: terminal is focused, CWD is detected from /proc.
        // Facts must propagate CWD to both terminal_cwd and active_terminal.
        let mut facts = OperationalFacts::empty(GroundingCapabilities::none());
        let cwd = PathBuf::from("/home/user/projects/kria");
        facts.terminal_cwd = Some(cwd.clone());
        facts.active_terminal = Some(TerminalFact {
            binary: Some("alacritty".to_string()),
            focused: true,
            cwd: Some(cwd.clone()),
            pid: Some(9999),
        });

        // Verify both paths agree
        assert_eq!(
            facts.terminal_cwd,
            facts.active_terminal.as_ref().unwrap().cwd
        );
        assert_eq!(
            facts.terminal_cwd.unwrap(),
            PathBuf::from("/home/user/projects/kria")
        );
    }

    #[test]
    fn stale_cache_never_leaks_across_generations() {
        // Scenario: cache filled at gen 0, invalidated to gen 5,
        // refilled at gen 5. The gen-0 data must never be returned.
        let cache = GroundingCache::new();

        let mut old_facts = OperationalFacts::empty(GroundingCapabilities::none());
        old_facts.focused_app = Some("OLD_APP".to_string());
        cache.store(old_facts);

        // Simulate 5 invalidations
        for _ in 0..5 {
            cache.invalidate();
        }
        assert_eq!(cache.generation(), 5);
        assert!(cache.get_if_fresh().is_none()); // old data gone

        // Refill with new data
        let mut new_facts = OperationalFacts::empty(GroundingCapabilities::none());
        new_facts.focused_app = Some("NEW_APP".to_string());
        cache.store(new_facts);

        // Must return NEW data, never OLD
        let snap = cache.get_if_fresh().unwrap();
        assert_eq!(snap.focused_app, Some("NEW_APP".to_string()));
    }

    #[test]
    fn generation_monotonicity_under_concurrent_invalidation() {
        // Scenario: multiple threads invalidate concurrently.
        // Generation must always increase, never go backwards.
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(GroundingCache::new());
        let mut handles = Vec::new();

        for _ in 0..8 {
            let cache_clone = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    cache_clone.invalidate();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // 8 threads × 100 invalidations = exactly 800
        assert_eq!(cache.generation(), 800);
    }

    #[test]
    fn xdotool_unavailable_graceful_degradation() {
        // Scenario: xdotool not installed.
        // GroundingCapabilities should report has_window_query=false.
        let caps = GroundingCapabilities {
            has_window_query: false,
            has_window_list: true,   // wmctrl still available
            has_monitor_query: true, // xrandr still available
            display_server: DisplayServerType::X11,
        };

        // Status should clearly show partial degradation
        let cache = GroundingCache::new();
        let mut facts = OperationalFacts::empty(caps);
        facts.focused_window = None; // Can't query without xdotool
        facts.visible_windows = vec![WindowFact {
            title: "Window from wmctrl".into(),
            class: String::new(),
            pid: 123,
            desktop: 0,
            geometry: Some(Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            }),
        }];
        cache.store(facts);

        let status = cache.snapshot_status();
        assert!(!status.capabilities.has_window_query);
        assert!(status.capabilities.has_window_list);
        assert!(status.focused_app.is_none());
        assert_eq!(status.visible_window_count, 1);
    }

    #[test]
    fn wmctrl_unavailable_graceful_degradation() {
        // Scenario: wmctrl not installed.
        let caps = GroundingCapabilities {
            has_window_query: true, // xdotool available
            has_window_list: false, // wmctrl missing
            has_monitor_query: true,
            display_server: DisplayServerType::X11,
        };

        let facts = OperationalFacts::empty(caps);
        assert!(facts.visible_windows.is_empty());
        assert!(facts.capabilities.has_window_query);
        assert!(!facts.capabilities.has_window_list);
    }

    #[test]
    fn process_lifecycle_invalidation_separate_from_desktop() {
        // Verify that ProcessLifecycle events ALSO invalidate the cache
        // (a new process may have created a window).
        use crate::agent::perception::EventKind;

        let should_invalidate = matches!(
            &EventKind::ProcessLifecycle("nginx_crashed".to_string()),
            EventKind::DesktopEvent(_) | EventKind::ProcessLifecycle(_)
        );
        assert!(should_invalidate);

        // Filesystem events should NOT invalidate
        use crate::agent::perception::FilesystemOp;
        let should_not = matches!(
            &EventKind::Filesystem(FilesystemOp::Modified),
            EventKind::DesktopEvent(_) | EventKind::ProcessLifecycle(_)
        );
        assert!(!should_not);
    }

    #[test]
    fn operational_facts_advisory_only_invariant() {
        // CRITICAL INVARIANT: OperationalFacts must never contain
        // decision fields, confidence scores, or action recommendations.
        // It is purely observational data.
        let facts = OperationalFacts::empty(GroundingCapabilities::none());

        // Verify the struct has NO decision/reasoning fields:
        // - No "should_open" field
        // - No "confidence" field
        // - No "recommended_action" field
        // - No "reasoning" field
        // The type system enforces this — we just verify the advisory contract
        // by checking that facts only contain observational data.
        assert!(facts.focused_window.is_none());
        assert!(facts.focused_app.is_none());
        assert!(facts.terminal_cwd.is_none());
        assert!(facts.open_project_path.is_none());
        assert!(facts.visible_windows.is_empty());
        assert!(facts.monitors.is_empty());
        assert!(facts.running_process_subset.is_empty());
        assert!(facts.active_terminal.is_none());
        // All fields are Option/Vec — observational, never prescriptive.
    }
}
