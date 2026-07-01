/// Installed application registry for Linux (freedesktop .desktop files).
///
/// Scans `.desktop` files from standard locations including system, user-local,
/// Snap, Flatpak exports, and AppImage-generated entries. Builds:
/// - A name→`CanonicalAppId` alias map for LLM input normalization.
/// - A `CanonicalAppId`→desktop-file-path map for `gio launch`.
/// - A set of registered URI schemes for deep-link classification.
/// - A fingerprint (SHA-256) of each `Exec=` line to detect handler hijacking.
///
/// # Refresh strategy
/// - Startup: full scan, blocking until complete.
/// - Runtime: `notify` filesystem watcher on all scan directories.
/// - Belt-and-suspenders: periodic full rescan every 5 minutes.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::platform::intent::capability::CanonicalAppId;

// ─── Scan directories (Linux) ─────────────────────────────────────────────────

/// All directories that may contain `.desktop` application entries on a Linux desktop.
static SCAN_DIRS: Lazy<Vec<PathBuf>> = Lazy::new(|| {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"));
    vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        home.join(".local/share/applications"),
        // Snap desktop integration.
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        // Flatpak: user-level.
        home.join(".local/share/flatpak/exports/share/applications"),
        // Flatpak: system-level.
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ]
});

// ─── AppManifest ─────────────────────────────────────────────────────────────

/// Metadata extracted from a `.desktop` file.
#[derive(Clone, Debug)]
pub struct AppManifest {
    /// Canonical application ID (usually the `.desktop` filename without extension,
    /// or the `StartupWMClass=` value if present).
    pub app_id: CanonicalAppId,
    /// Human-readable display name from `Name=`.
    pub display_name: String,
    /// Path to the `.desktop` file — used by `gio launch`.
    pub desktop_path: PathBuf,
    /// `Exec=` line value — used for fingerprinting.
    pub exec_line: String,
    /// SHA-256 of the `Exec=` line at discovery time.
    /// Compared on every launch to detect default-handler hijacking.
    pub exec_fingerprint: [u8; 32],
    /// URI schemes registered by this app (from `MimeType=x-scheme-handler/<scheme>`).
    pub registered_schemes: Vec<String>,
    /// Additional SPECIFIC name aliases (from `Name=`, reverse-DNS last segment,
    /// `X-KDE-Aliases=`, etc.) used for fuzzy resolution of LLM-supplied names
    /// like "chrome" → "chromium". These identify THIS app and may override
    /// built-ins / earlier entries (deterministic last-writer-wins for visible
    /// apps).
    pub name_aliases: Vec<String>,
    /// GENERIC, category-level aliases from `GenericName=` (e.g. "text editor",
    /// "web browser", "file manager"). These are SHARED by many unrelated apps —
    /// heavyweight IDEs (`code`, `kiro`, `devin-desktop`, `vim`) all advertise
    /// `GenericName=Text Editor`. They must NEVER clobber a built-in class alias
    /// or a dedicated app's claim, or "open the text editor" would launch
    /// whichever IDE happened to be scanned last (the live OpenApp bug). Under
    /// the `gui_cog_verify_live` flag these are registered NON-clobbering
    /// (`or_insert`). See `load_manifests`.
    pub generic_aliases: Vec<String>,
    /// `true` when the `.desktop` entry sets `NoDisplay=true` — a hidden helper
    /// (e.g. a `x-scheme-handler/*` URL-handler stub) that is launchable by its
    /// own ID but should NEVER hijack a human-facing class alias ("text editor",
    /// "editor", …) away from a real, visible application. See `load_manifests`.
    pub no_display: bool,
}

/// Outcome of a fuzzy app-name match (Requirement 6: mistyped/ambiguous/closest).
#[derive(Debug, Clone, PartialEq)]
pub enum AppMatch {
    /// One confident match — `alias` is a registry alias key (resolve it to an id).
    Closest { alias: String, score: f32 },
    /// Several plausible matches — ask the user which one.
    Ambiguous(Vec<String>),
    /// No confident match — nearest suggestions for an honest "not installed" reply.
    None(Vec<String>),
}

/// Similarity score in [0,1] between a query and a candidate app name. Combines
/// substring containment (strong) with a normalized edit-distance ratio so both
/// typos ("chrohme"→"chrome") and partials ("explorer"→"file explorer") score high.
fn name_match_score(query: &str, candidate: &str) -> f32 {
    if query == candidate {
        return 1.0;
    }
    let lev = levenshtein_ratio(query, candidate);
    // Containment only counts when the SHORTER string is substantial (≥4 chars)
    // and the two are length-comparable — otherwise a short alias that happens to
    // be a substring (e.g. "bar" inside "foobar123") would falsely match.
    let shorter = query.len().min(candidate.len());
    let contains = (candidate.contains(query) || query.contains(candidate)) && shorter >= 4;
    if contains {
        let len_ratio = (shorter as f32) / (query.len().max(candidate.len()) as f32);
        if len_ratio >= 0.5 {
            return (0.88 + 0.12 * len_ratio).max(lev);
        }
    }
    lev
}

/// Normalized Levenshtein ratio in [0,1] (1.0 == identical).
fn levenshtein_ratio(a: &str, b: &str) -> f32 {
    let max = a.chars().count().max(b.chars().count());
    if max == 0 {
        return 1.0;
    }
    let dist = levenshtein(a, b);
    1.0 - (dist as f32 / max as f32)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// ─── InstalledAppRegistry ─────────────────────────────────────────────────────
/// Thread-safe, self-refreshing registry of installed desktop applications.
pub struct InstalledAppRegistry {
    /// `CanonicalAppId.as_str()` → `AppManifest`
    apps: Arc<RwLock<HashMap<String, AppManifest>>>,
    /// Lowercase name/alias → `CanonicalAppId.as_str()`
    aliases: Arc<RwLock<HashMap<String, String>>>,
    /// URI scheme string → `CanonicalAppId.as_str()`
    schemes: Arc<RwLock<HashMap<String, String>>>,
}

impl InstalledAppRegistry {
    /// Build the registry synchronously. Suitable for calling at startup before the
    /// Tokio runtime is needed for other work.
    pub fn build_sync() -> Arc<Self> {
        let registry = Arc::new(Self {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });

        // Perform an immediate full scan on the current thread.
        let manifests = scan_all_desktop_files();
        let rt = tokio::runtime::Handle::try_current();
        if let Ok(handle) = rt {
            handle.block_on(registry.load_manifests(manifests));
        } else {
            // No runtime yet — we'll populate lazily on first access.
            // This path should not happen in production but is safe.
            warn!("InstalledAppRegistry::build_sync called without a Tokio runtime");
        }

        registry
    }

    /// Build the registry asynchronously. The registry is empty until
    /// `initialize()` completes.
    pub async fn build_async() -> Arc<Self> {
        let registry = Arc::new(Self {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        registry.initialize().await;
        registry
    }

    /// Full scan and load. Idempotent — replaces existing data.
    pub async fn initialize(&self) {
        info!("InstalledAppRegistry: starting full scan");
        let manifests = tokio::task::spawn_blocking(scan_all_desktop_files)
            .await
            .unwrap_or_default();
        info!(
            count = manifests.len(),
            "InstalledAppRegistry: scan complete"
        );
        self.load_manifests(manifests).await;
    }

    async fn load_manifests(&self, manifests: Vec<AppManifest>) {
        self.load_manifests_with_guard(manifests, generic_alias_guard_enabled())
            .await
    }

    /// Issue #2 (live OpenApp): load manifests with explicit control over the
    /// generic-category-alias guard, so tests are deterministic and don't race
    /// on the process-global env var. `guard_enabled == true` (the desktop
    /// default-on path) registers `GenericName`-derived aliases NON-clobbering
    /// so a shared category like "text editor" cannot be hijacked away from a
    /// built-in / dedicated app by a heavyweight IDE. `false` restores the prior
    /// byte-for-byte behavior (generic aliases clobber like specific ones).
    async fn load_manifests_with_guard(&self, manifests: Vec<AppManifest>, guard_enabled: bool) {
        let mut apps = self.apps.write().await;
        let mut aliases = self.aliases.write().await;
        let mut schemes = self.schemes.write().await;

        apps.clear();
        aliases.clear();
        schemes.clear();

        // Seed with built-in aliases first so .desktop entries can override them.
        // This ensures "chrome", "vscode", "terminal" etc. resolve even on systems
        // where the .desktop Name= field doesn't match the common user-facing name.
        for (alias, id) in builtin_alias_map() {
            aliases.insert(alias, id);
        }

        for manifest in manifests {
            let id_str = manifest.app_id.as_str().to_lowercase();

            // The canonical-ID self-alias is always authoritative (it points to
            // the app itself, so it can never shadow a different app).
            aliases.insert(id_str.clone(), id_str.clone());

            // Human-facing aliases (display name + GenericName/reverse-DNS).
            // A `NoDisplay=true` helper (e.g. a `x-scheme-handler/*` URL-handler
            // stub such as `devin-desktop-url-handler` with `GenericName=Text
            // Editor`) must NEVER overwrite a human class alias ("text editor",
            // "editor", …) that already resolves to a real, visible app — doing so
            // makes `gio launch` run a URL handler that exits immediately instead
            // of opening the editor. Hidden helpers therefore only register an
            // alias when it is not already claimed; visible apps keep overriding
            // built-ins and earlier entries as before (deterministic: a real app
            // always wins over a hidden helper regardless of scan order).
            let mut register_alias = |alias: String, force_no_clobber: bool| {
                if alias.is_empty() {
                    return;
                }
                if manifest.no_display || force_no_clobber {
                    aliases.entry(alias).or_insert_with(|| id_str.clone());
                } else {
                    aliases.insert(alias, id_str.clone());
                }
            };
            register_alias(manifest.display_name.to_lowercase(), false);
            for alias in &manifest.name_aliases {
                register_alias(alias.to_lowercase(), false);
            }

            // GenericName-derived category aliases ("text editor", "web browser",
            // …) are SHARED across many unrelated apps. When the guard is ON they
            // are registered NON-clobbering so they can only FILL an unclaimed
            // alias — never steal a built-in class alias or a dedicated app's
            // claim. This is what stops "open the text editor" from launching
            // whichever IDE (`code`/`kiro`/`devin-desktop`/`vim`, all of which set
            // `GenericName=Text Editor`) happened to be scanned last instead of
            // the real GNOME Text Editor (the live OpenApp 20ms-no-op bug). When
            // the guard is OFF the prior behavior is preserved byte-for-byte:
            // generic aliases clobber exactly like specific ones did.
            for alias in &manifest.generic_aliases {
                register_alias(alias.to_lowercase(), guard_enabled);
            }

            // Register URI schemes (hidden URL-handlers are still valid scheme
            // owners, so this is unaffected by the alias guard above).
            for scheme in &manifest.registered_schemes {
                schemes.insert(scheme.clone(), id_str.clone());
            }

            apps.insert(id_str, manifest);
        }

        debug!(
            apps = apps.len(),
            aliases = aliases.len(),
            "InstalledAppRegistry: loaded manifests with built-in aliases"
        );
    }

    /// Spawn a background task that watches scan directories for changes and
    /// performs a periodic full rescan every 5 minutes.
    pub fn spawn_watcher(self: Arc<Self>) {
        let registry = Arc::clone(&self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            interval.tick().await; // First tick fires immediately; skip it.
            loop {
                interval.tick().await;
                debug!("InstalledAppRegistry: periodic rescan");
                registry.initialize().await;
            }
        });
    }

    /// Check whether an app with the given `CanonicalAppId` is installed.
    pub fn is_installed(&self, app_id: &CanonicalAppId) -> bool {
        // We need a blocking read in a sync context; use try_read.
        if let Ok(apps) = self.apps.try_read() {
            apps.contains_key(&app_id.as_str().to_lowercase())
        } else {
            // FIX #14: Fail CLOSED on lock contention, not open.
            // The previous behavior (return true) allowed any app name to pass
            // the installed check during the 5-minute periodic rescan, which
            // could allow an LLM to launch arbitrary binaries.
            // Failing closed means the dispatch will be rejected with "not found"
            // which is safer than silently allowing an unverified launch.
            warn!(
                app_id = %app_id.as_str(),
                "InstalledAppRegistry: lock contention in is_installed — failing closed"
            );
            false
        }
    }

    /// Get the `.desktop` file path for a canonical app ID.
    pub fn desktop_path(&self, app_id: &CanonicalAppId) -> Option<PathBuf> {
        let apps = self.apps.try_read().ok()?;
        apps.get(&app_id.as_str().to_lowercase())
            .map(|m| m.desktop_path.clone())
    }

    /// Resolve a user-supplied app name (e.g., "chrome", "Google Chrome") to a
    /// `CanonicalAppId`. Returns `None` if no match.
    ///
    /// Uses `try_read()` to avoid blocking the caller. On lock contention (rare,
    /// only during the 5-minute periodic rescan), logs a warning and returns `None`
    /// rather than silently failing. Callers should treat `None` as "not found"
    /// and surface an appropriate error.
    pub fn resolve_alias(&self, name: &str) -> Option<CanonicalAppId> {
        match self.aliases.try_read() {
            Ok(aliases) => {
                let normalized = normalize_alias(name);

                // Try original normalized first (exact match)
                if let Some(id_str) = aliases.get(&normalized) {
                    return Some(CanonicalAppId::from_registry(id_str.clone()));
                }

                // Try filler-word-stripped candidates (handles "the settings app" → "settings")
                let stripped_candidates = strip_filler_words(&normalized);
                for candidate in &stripped_candidates {
                    if candidate == &normalized {
                        continue; // already tried above
                    }
                    if let Some(id_str) = aliases.get(candidate) {
                        debug!(
                            target: "app_registry",
                            input = %name,
                            normalized = %normalized,
                            matched_candidate = %candidate,
                            "Resolved alias via filler-word stripping"
                        );
                        return Some(CanonicalAppId::from_registry(id_str.clone()));
                    }
                }

                if let Ok(apps) = self.apps.try_read() {
                    // Try class-alias resolution on each candidate
                    for candidate in &stripped_candidates {
                        if let Some(id_str) = resolve_class_alias(candidate, &aliases, &apps) {
                            return Some(CanonicalAppId::from_registry(id_str));
                        }
                    }
                }

                None
            }
            Err(_) => {
                warn!(
                    app_name = name,
                    "InstalledAppRegistry: alias lock contention during resolve — \
                     registry is being rescanned. Retry the request."
                );
                None
            }
        }
    }

    /// Fuzzy-match a user-supplied app name against installed apps + aliases for
    /// robust resolution when an exact alias lookup fails (Requirement 6:
    /// mistyped / synonym / closest-match). Returns a confident `Closest`, an
    /// `Ambiguous` set, or `None` with nearest suggestions for an honest reply.
    /// Pure scoring (containment + edit distance); no I/O beyond a try_read.
    pub fn fuzzy_match(&self, name: &str) -> AppMatch {
        let query = name.trim().to_ascii_lowercase();
        if query.is_empty() {
            return AppMatch::None(Vec::new());
        }
        // Candidate (label → canonical app id) pairs from alias keys + display names.
        let mut labeled: Vec<(String, String)> = Vec::new();
        if let Ok(aliases) = self.aliases.try_read() {
            for (alias, id) in aliases.iter() {
                labeled.push((alias.clone(), id.to_ascii_lowercase()));
            }
        }
        if let Ok(apps) = self.apps.try_read() {
            for (id, m) in apps.iter() {
                labeled.push((m.display_name.to_ascii_lowercase(), id.clone()));
            }
        }
        if labeled.is_empty() {
            return AppMatch::None(Vec::new());
        }
        // Score each label, then GROUP BY canonical id keeping the best score +
        // label, so multiple aliases of the SAME app are one candidate (not
        // false-ambiguous, e.g. "explorer"/"file explorer"/"files" → one app).
        let mut best_by_id: HashMap<String, (String, f32)> = HashMap::new();
        for (label, id) in labeled {
            let score = name_match_score(&query, &label);
            best_by_id
                .entry(id)
                .and_modify(|e| {
                    if score > e.1 {
                        *e = (label.clone(), score);
                    }
                })
                .or_insert((label, score));
        }
        let mut scored: Vec<(String, f32)> = best_by_id.into_values().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        const STRONG: f32 = 0.84;
        const WEAK: f32 = 0.45;
        let top = scored.first().cloned().unwrap_or_default();
        let strong: Vec<String> = scored
            .iter()
            .filter(|(_, s)| *s >= STRONG)
            .map(|(c, _)| c.clone())
            .collect();
        if top.1 >= STRONG {
            let second = scored.get(1).map(|(_, s)| *s).unwrap_or(0.0);
            if strong.len() == 1 || top.1 - second >= 0.08 {
                return AppMatch::Closest {
                    alias: top.0,
                    score: top.1,
                };
            }
            return AppMatch::Ambiguous(strong.into_iter().take(4).collect());
        }
        let suggestions: Vec<String> = scored
            .into_iter()
            .filter(|(_, s)| *s >= WEAK)
            .take(3)
            .map(|(c, _)| c)
            .collect();
        AppMatch::None(suggestions)
    }

    /// Return all URI schemes registered by installed applications.
    pub fn registered_schemes(&self) -> HashSet<String> {
        if let Ok(schemes) = self.schemes.try_read() {
            schemes.keys().cloned().collect()
        } else {
            HashSet::new()
        }
    }

    /// Check whether a specific app's `Exec=` line matches its registered fingerprint.
    /// Returns `true` if the fingerprint is valid (or if fingerprinting is unavailable).
    /// Returns `false` if the handler binary appears to have changed — callers should
    /// elevate to RED and warn the user.
    pub fn verify_exec_fingerprint(&self, app_id: &CanonicalAppId) -> bool {
        let apps = match self.apps.try_read() {
            Ok(a) => a,
            Err(_) => return true, // fail open on lock contention
        };
        let manifest = match apps.get(&app_id.as_str().to_lowercase()) {
            Some(m) => m,
            None => return true,
        };

        // Re-hash the current Exec= value from the .desktop file on disk.
        let current_exec = read_exec_line(&manifest.desktop_path);
        match current_exec {
            None => true, // Can't read — assume valid.
            Some(exec) => {
                let current_hash = sha256_bytes(exec.as_bytes());
                current_hash == manifest.exec_fingerprint
            }
        }
    }
}

fn normalize_alias(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        // Treat underscores as spaces: LLM/tool callers often snake_case app
        // names (e.g. "files_manager"), but registry aliases are space-separated
        // ("files manager"). Normalizing here lets both forms resolve identically.
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strip common filler words ("the", "a", "an", "app", "application", "program", "tool")
/// from an alias to enable natural-language phrasing like "the Settings app" → "settings".
///
/// Returns a list of progressively-stripped candidates, ordered from least-modified
/// to most-modified. Callers should try each in order.
fn strip_filler_words(normalized: &str) -> Vec<String> {
    let filler_words: &[&str] = &[
        "the",
        "a",
        "an",
        "my",
        "app",
        "application",
        "applications",
        "program",
        "programs",
        "tool",
        "tools",
        "software",
    ];

    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.is_empty() {
        return vec![normalized.to_string()];
    }

    let mut candidates: Vec<String> = Vec::new();
    candidates.push(normalized.to_string()); // original first

    // Strip filler words and produce a stripped candidate
    let stripped: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| !filler_words.contains(&t.to_ascii_lowercase().as_str()))
        .collect();

    if !stripped.is_empty() && stripped.len() != tokens.len() {
        candidates.push(stripped.join(" "));
    }

    // Also try just the first non-filler token (e.g., "settings app" → "settings")
    if let Some(first) = stripped.first() {
        let single = first.to_string();
        if !candidates.contains(&single) {
            candidates.push(single);
        }
    }

    // Also try the last non-filler token (e.g., "the file manager" → "manager" already
    // handled by stripped; but "the chrome browser" → "browser" might be wrong, so
    // we prefer first-token. Skip last-token strategy for now to avoid false matches.)

    candidates
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn strip_filler_settings_app() {
        let candidates = strip_filler_words("settings app");
        assert!(candidates.contains(&"settings app".to_string()));
        assert!(candidates.contains(&"settings".to_string()));
    }

    #[test]
    fn strip_filler_the_settings_app() {
        let candidates = strip_filler_words("the settings app");
        assert!(candidates.contains(&"settings".to_string()));
    }

    #[test]
    fn strip_filler_my_browser() {
        let candidates = strip_filler_words("my browser");
        assert!(candidates.contains(&"browser".to_string()));
    }

    #[test]
    fn strip_filler_no_change() {
        let candidates = strip_filler_words("chrome");
        assert_eq!(candidates, vec!["chrome".to_string()]);
    }

    #[test]
    fn normalize_alias_treats_underscore_as_space() {
        // LLM/tool callers often snake_case the app name; it must normalize to
        // the same space-separated form the registry aliases use.
        assert_eq!(normalize_alias("files_manager"), "files manager");
        assert_eq!(normalize_alias("system_settings"), "system settings");
        assert_eq!(normalize_alias("  text__editor  "), "text editor");
    }

    #[test]
    fn underscored_file_manager_maps_to_candidates() {
        // The exact failure seen live: Brain emitted OpenApp{app:"files_manager"}.
        let normalized = normalize_alias("files_manager");
        assert!(
            class_alias_candidates(&normalized).is_some_and(|c| c.contains(&"nautilus")),
            "normalized {normalized:?} should map to the file-manager candidate set"
        );
    }

    #[test]
    fn file_manager_aliases_resolve_to_candidates() {
        for name in [
            "file manager",
            "files manager",
            "file browser",
            "file explorer",
            "files app",
        ] {
            assert!(
                class_alias_candidates(name).is_some_and(|c| c.contains(&"nautilus")),
                "{name:?} should map to the file-manager candidate set"
            );
        }
    }
}

fn resolve_class_alias(
    normalized: &str,
    aliases: &HashMap<String, String>,
    apps: &HashMap<String, AppManifest>,
) -> Option<String> {
    let candidates = class_alias_candidates(normalized)?;
    for candidate in candidates {
        let candidate = normalize_alias(candidate);
        if let Some(id) = aliases.get(&candidate) {
            if apps.contains_key(&id.to_ascii_lowercase()) {
                return Some(id.clone());
            }
        }
        if apps.contains_key(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn class_alias_candidates(normalized: &str) -> Option<&'static [&'static str]> {
    const SPREADSHEET: &[&str] = &[
        "libreoffice calc",
        "libreoffice-calc",
        "calc",
        "gnumeric",
        "onlyoffice desktop editors",
        "wps spreadsheets",
    ];
    const TEXT_EDITOR: &[&str] = &[
        "text editor",
        "gnome text editor",
        "gedit",
        "kate",
        "xed",
        "mousepad",
    ];
    const FILE_MANAGER: &[&str] = &[
        "files",
        "file manager",
        "nautilus",
        "dolphin",
        "thunar",
        "nemo",
    ];
    const IDE: &[&str] = &[
        "code",
        "visual studio code",
        "vscode",
        "vscodium",
        "cursor",
        "windsurf",
        "zed",
    ];

    if matches!(
        normalized,
        "excel"
            | "microsoft excel"
            | "excel or calc"
            | "calc or excel"
            | "spreadsheet"
            | "spreadsheet app"
            | "sheet app"
            | "libreoffice calc"
            | "calc"
    ) {
        Some(SPREADSHEET)
    } else if matches!(
        normalized,
        "text editor"
            | "editor"
            | "plain text editor"
            | "document editor"
            | "notepad"
            | "text edit"
    ) {
        Some(TEXT_EDITOR)
    } else if matches!(
        normalized,
        "file manager"
            | "files manager"
            | "files"
            | "files app"
            | "file browser"
            | "files browser"
            | "file explorer"
            | "files explorer"
            | "home folder"
            | "folder viewer"
    ) {
        Some(FILE_MANAGER)
    } else if matches!(
        normalized,
        "code" | "code editor" | "ide" | "editor for code" | "code ide"
    ) {
        Some(IDE)
    } else {
        None
    }
}

// ─── Desktop file parsing ─────────────────────────────────────────────────────

/// Issue #2 (live OpenApp): whether the generic-category-alias guard is active.
///
/// Shares the Phase 1 `gui_cog_verify_live` flag (`KRIA_GUI_COG_VERIFY_LIVE`):
/// the registry's "GenericName must not hijack a class alias" fix ships with the
/// same live-verification wave. Default-ON (absent var ⇒ ON, matching the
/// desktop's `from_env_default_on` wiring); an explicit falsy value
/// (`0`/`false`/`no`/`off`/empty) is the documented rollback to the prior
/// clobbering behavior.
fn generic_alias_guard_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_VERIFY_LIVE") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

fn scan_all_desktop_files() -> Vec<AppManifest> {
    let mut manifests = Vec::new();

    for dir in SCAN_DIRS.iter() {
        if !dir.exists() {
            continue;
        }
        match std::fs::read_dir(dir) {
            Err(e) => {
                debug!("cannot read scan dir {}: {e}", dir.display());
                continue;
            }
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("desktop") {
                        if let Some(manifest) = parse_desktop_file(&path) {
                            manifests.push(manifest);
                        }
                    }
                }
            }
        }
    }

    manifests
}

fn parse_desktop_file(path: &Path) -> Option<AppManifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut exec = String::new();
    let mut generic_name = String::new();
    let mut startup_wm_class = String::new();
    let mut mime_types: Vec<String> = Vec::new();
    let mut hidden = false;
    let mut no_display = false;

    for line in content.lines() {
        // FIX #13: Trim \r to handle Windows-formatted .desktop files (CRLF line endings).
        // Without this, "[Desktop Entry]\r" != "[Desktop Entry]" and the parser breaks
        // immediately, silently skipping the entire manifest.
        let line = line.trim_end_matches('\r');

        // Only parse the [Desktop Entry] section.
        if line.starts_with('[') && line != "[Desktop Entry]" {
            break;
        }
        if let Some(val) = line.strip_prefix("Name=") {
            if name.is_empty() {
                name = val.trim().to_string();
            }
        } else if let Some(val) = line.strip_prefix("Exec=") {
            exec = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("GenericName=") {
            generic_name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("StartupWMClass=") {
            startup_wm_class = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("MimeType=") {
            // e.g. "application/pdf;x-scheme-handler/https;x-scheme-handler/http;"
            for part in val.split(';') {
                let part = part.trim();
                if part.starts_with("x-scheme-handler/") {
                    if let Some(scheme) = part.strip_prefix("x-scheme-handler/") {
                        mime_types.push(scheme.to_string());
                    }
                }
            }
        } else if line == "Hidden=true" || line == "NoDisplay=true" {
            if line == "Hidden=true" {
                hidden = true;
            }
            if line == "NoDisplay=true" {
                no_display = true;
            }
            // NoDisplay=true: still index the app (it's launchable by name)
            // but don't add it to the visible launcher list.
        }
    }

    // Index NoDisplay=true apps (they're still launchable by name, just hidden
    // from application launchers). Only skip truly Hidden=true entries.
    if name.is_empty() || hidden {
        return None;
    }

    // Derive canonical app ID: StartupWMClass > filename stem.
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let canonical_id = if !startup_wm_class.is_empty() {
        startup_wm_class.to_lowercase()
    } else {
        file_stem
    };

    let exec_fingerprint = sha256_bytes(exec.as_bytes());

    // Build SPECIFIC aliases from display name (reverse-DNS added below). The
    // GenericName is a SHARED category label and is kept separate so it can be
    // registered non-clobbering (see `load_manifests`).
    let mut aliases = vec![name.to_lowercase()];
    let mut generic_aliases: Vec<String> = Vec::new();
    if !generic_name.is_empty() {
        generic_aliases.push(generic_name.to_lowercase());
    }

    // Auto-extract Flatpak/reverse-DNS aliases.
    // For IDs like "org.gnome.gedit", "com.visualstudio.code", etc.,
    // add the last segment as an alias so users can say "gedit", "code", etc.
    // This handles Flatpak-only installs where the .desktop stem is the full
    // reverse-DNS ID rather than the short binary name.
    if canonical_id.contains('.') {
        let segments: Vec<&str> = canonical_id.split('.').collect();
        if segments.len() >= 2 {
            let last = segments.last().unwrap_or(&"").to_lowercase();
            if !last.is_empty() && last.len() > 1 {
                aliases.push(last.clone());
                // Also add hyphenated variant (e.g., "TextEditor" → "text-editor")
                let hyphenated = last
                    .chars()
                    .enumerate()
                    .flat_map(|(i, c)| {
                        if i > 0 && c.is_uppercase() {
                            vec!['-', c.to_ascii_lowercase()]
                        } else {
                            vec![c.to_ascii_lowercase()]
                        }
                    })
                    .collect::<String>();
                if hyphenated != last {
                    aliases.push(hyphenated);
                }
            }
        }
    }

    Some(AppManifest {
        app_id: CanonicalAppId::from_registry(canonical_id),
        display_name: name,
        desktop_path: path.to_path_buf(),
        exec_line: exec,
        exec_fingerprint,
        registered_schemes: mime_types,
        name_aliases: aliases,
        generic_aliases,
        no_display,
    })
}

fn read_exec_line(desktop_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(desktop_path).ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("Exec=") {
            return Some(val.trim().to_string());
        }
    }
    None
}

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

// ─── Well-known alias overrides ───────────────────────────────────────────────
//
// These supplement the .desktop scanner with common user-facing names that
// don't always appear in the Name= field.
//
// This replaces the ad-hoc `match app_name` block in loop_engine.rs L216-221.

pub fn builtin_alias_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    let pairs: &[(&str, &str)] = &[
        // Chrome: map to google-chrome-stable (the canonical installed name on most systems).
        // On Chromium-only systems the .desktop scanner adds "chromium" from its .desktop file.
        ("chrome", "google-chrome-stable"),
        ("google chrome", "google-chrome-stable"),
        ("google-chrome", "google-chrome-stable"),
        ("google-chrome-stable", "google-chrome-stable"),
        ("chromium", "chromium"),
        ("chromium-browser", "chromium"),
        ("chrome browser", "google-chrome-stable"),
        ("google chrome browser", "google-chrome-stable"),
        // Firefox
        ("firefox", "firefox"),
        ("ff", "firefox"),
        ("mozilla firefox", "firefox"),
        // VS Code
        ("vscode", "code"),
        ("visual studio code", "code"),
        ("vs code", "code"),
        ("code-oss", "code-oss"),
        ("vscodium", "vscodium"),
        // Editors
        ("gedit", "gedit"),
        ("kate", "kate"),
        ("mousepad", "mousepad"),
        ("xed", "xed"),
        ("gnome text editor", "org.gnome.TextEditor"),
        ("text editor", "org.gnome.TextEditor"),
        ("plain text editor", "org.gnome.TextEditor"),
        // Terminals
        ("terminal", "org.gnome.Terminal"),
        ("gnome terminal", "org.gnome.Terminal"),
        ("gnome-terminal", "org.gnome.Terminal"),
        ("konsole", "org.kde.konsole"),
        ("xfce terminal", "xfce4-terminal"),
        ("alacritty", "Alacritty"),
        ("kitty", "kitty"),
        // Messaging
        ("whatsapp", "whatsapp-linux-amd64"),
        ("telegram", "telegramdesktop"),
        ("signal", "signal-desktop"),
        // Files
        ("files", "org.gnome.Nautilus"),
        ("file manager", "org.gnome.Nautilus"),
        ("file explorer", "org.gnome.Nautilus"),
        ("explorer", "org.gnome.Nautilus"),
        ("file browser", "org.gnome.Nautilus"),
        ("nautilus", "org.gnome.Nautilus"),
        ("thunar", "thunar"),
        ("dolphin", "org.kde.dolphin"),
        // Spreadsheets. Class aliases are resolved against installed apps first;
        // these direct aliases keep common user names deterministic when Calc is installed.
        ("spreadsheet", "libreoffice-calc"),
        ("spreadsheet app", "libreoffice-calc"),
        ("excel", "libreoffice-calc"),
        ("microsoft excel", "libreoffice-calc"),
        ("excel or calc", "libreoffice-calc"),
        ("calc or excel", "libreoffice-calc"),
        ("libreoffice calc", "libreoffice-calc"),
        // System
        ("calculator", "org.gnome.Calculator"),
        ("settings", "gnome-control-center"),
        ("system settings", "systemsettings"),
        // Brave
        ("brave", "brave-browser"),
        ("brave browser", "brave-browser"),
        // Edge
        ("edge", "microsoft-edge"),
        ("microsoft edge", "microsoft-edge"),
        // Flatpak common reverse-DNS aliases
        ("org.gnome.gedit", "org.gnome.gedit"),
        ("org.gnome.nautilus", "org.gnome.Nautilus"),
        ("org.kde.kate", "org.kde.kate"),
        ("com.visualstudio.code", "code"),
        ("com.google.chrome", "google-chrome-stable"),
        ("org.mozilla.firefox", "firefox"),
    ];
    for (alias, id) in pairs {
        m.insert(alias.to_string(), id.to_string());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_aliases_contain_chrome_variants() {
        let map = builtin_alias_map();
        // "chrome" maps to "google-chrome-stable" (not "chromium")
        // so it works on systems with Google Chrome installed.
        // On Chromium-only systems, the .desktop scanner will add "chromium"
        // as an alias from the Chromium .desktop file.
        assert_eq!(map.get("chrome").unwrap(), "google-chrome-stable");
        assert_eq!(map.get("google chrome").unwrap(), "google-chrome-stable");
        assert_eq!(map.get("google-chrome").unwrap(), "google-chrome-stable");
        // Chromium is also directly aliased
        assert_eq!(map.get("chromium").unwrap(), "chromium");
    }

    #[tokio::test]
    async fn class_alias_resolves_excel_or_calc_to_installed_spreadsheet() {
        let registry = Arc::new(InstalledAppRegistry {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        registry
            .load_manifests(vec![AppManifest {
                app_id: CanonicalAppId::from_registry("libreoffice-calc".to_string()),
                display_name: "LibreOffice Calc".to_string(),
                desktop_path: PathBuf::from("/tmp/libreoffice-calc.desktop"),
                exec_line: "libreoffice --calc %U".to_string(),
                exec_fingerprint: sha256_bytes(b"libreoffice --calc %U"),
                registered_schemes: Vec::new(),
                name_aliases: vec!["calc".to_string()],
                generic_aliases: Vec::new(),
                no_display: false,
            }])
            .await;

        let resolved = registry
            .resolve_alias("Excel or Calc")
            .expect("spreadsheet class alias should resolve");
        assert_eq!(resolved.as_str(), "libreoffice-calc");
    }

    #[tokio::test]
    async fn fuzzy_match_handles_typo_partial_ambiguous_and_unknown() {
        let registry = Arc::new(InstalledAppRegistry {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        let mk = |id: &str, name: &str, aliases: Vec<String>| AppManifest {
            app_id: CanonicalAppId::from_registry(id.to_string()),
            display_name: name.to_string(),
            desktop_path: PathBuf::from(format!("/tmp/{id}.desktop")),
            exec_line: id.to_string(),
            exec_fingerprint: sha256_bytes(id.as_bytes()),
            registered_schemes: Vec::new(),
            name_aliases: aliases,
            generic_aliases: Vec::new(),
            no_display: false,
        };
        registry
            .load_manifests(vec![
                mk("google-chrome", "Google Chrome", vec!["chrome".into()]),
                mk(
                    "org.gnome.Nautilus",
                    "Files",
                    vec!["file explorer".into(), "file manager".into()],
                ),
                mk("code", "Visual Studio Code", vec!["code".into()]),
                // A short alias that is a substring of "foobar123" — must NOT match.
                mk("org.x.bar", "Bar", vec!["bar".into()]),
            ])
            .await;

        // Typo → closest (chrome).
        match registry.fuzzy_match("chrohme") {
            AppMatch::Closest { alias, .. } => assert!(alias.contains("chrome")),
            other => panic!("expected Closest for typo, got {other:?}"),
        }
        // Partial → closest (file explorer → nautilus alias).
        match registry.fuzzy_match("explorer") {
            AppMatch::Closest { alias, .. } => {
                assert!(
                    registry.resolve_alias(&alias).is_some(),
                    "closest must resolve"
                );
            }
            other => panic!("expected Closest for partial, got {other:?}"),
        }
        // Unknown → None with (possibly empty) suggestions, never a wrong match.
        match registry.fuzzy_match("foobar123xyz") {
            AppMatch::None(_) => {}
            other => panic!("expected None for unknown app, got {other:?}"),
        }
        // A query containing a SHORT alias substring ("bar") must NOT false-match.
        match registry.fuzzy_match("foobar123") {
            AppMatch::None(_) => {}
            other => panic!("'foobar123' must not match short alias 'bar', got {other:?}"),
        }
    }

    #[tokio::test]
    async fn filler_words_stripped_settings_app_resolves_to_settings() {
        let registry = Arc::new(InstalledAppRegistry {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        registry
            .load_manifests(vec![AppManifest {
                app_id: CanonicalAppId::from_registry("gnome-control-center".to_string()),
                display_name: "Settings".to_string(),
                desktop_path: PathBuf::from("/tmp/gnome-control-center.desktop"),
                exec_line: "gnome-control-center".to_string(),
                exec_fingerprint: sha256_bytes(b"gnome-control-center"),
                registered_schemes: Vec::new(),
                name_aliases: vec!["settings".to_string()],
                generic_aliases: Vec::new(),
                no_display: false,
            }])
            .await;

        // Direct match works
        assert!(registry.resolve_alias("settings").is_some());
        // Filler-word stripping works
        assert!(
            registry.resolve_alias("settings app").is_some(),
            "'settings app' should resolve to 'settings' after stripping filler word"
        );
        assert!(
            registry.resolve_alias("the settings app").is_some(),
            "'the settings app' should resolve to 'settings'"
        );
        assert!(
            registry.resolve_alias("the settings application").is_some(),
            "'the settings application' should resolve to 'settings'"
        );
        assert!(
            registry.resolve_alias("settings program").is_some(),
            "'settings program' should resolve to 'settings'"
        );
    }

    #[test]
    fn sha256_produces_32_bytes() {
        let hash = sha256_bytes(b"test");
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn fingerprint_differs_on_changed_exec() {
        let h1 = sha256_bytes(b"Exec=/usr/bin/chromium %U");
        let h2 = sha256_bytes(b"Exec=/tmp/malicious %U");
        assert_ne!(h1, h2);
    }

    /// Regression: a `NoDisplay=true` URL-handler stub (e.g.
    /// `devin-desktop-url-handler` with `GenericName=Text Editor` + an
    /// `x-scheme-handler/*` Exec) must NOT hijack the human-facing "text editor"
    /// / "editor" aliases away from the real, visible GNOME Text Editor. If it
    /// did, `gio launch` would run the handler stub (which exits immediately with
    /// no URL) and no editor process would ever spawn — the live OpenApp bug.
    #[tokio::test]
    async fn editor_aliases_resolve_to_real_editor_not_hidden_url_handler() {
        let registry = Arc::new(InstalledAppRegistry {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        // Order the hidden handler FIRST so the guard (not scan order) is what
        // protects the alias.
        registry
            .load_manifests(vec![
                AppManifest {
                    app_id: CanonicalAppId::from_registry("devin-desktop-url-handler".to_string()),
                    display_name: "Devin - URL Handler".to_string(),
                    desktop_path: PathBuf::from(
                        "/usr/share/applications/devin-desktop-url-handler.desktop",
                    ),
                    exec_line: "/usr/share/devin-desktop/devin-desktop --open-url %U".to_string(),
                    exec_fingerprint: sha256_bytes(b"devin --open-url %U"),
                    registered_schemes: vec!["devin".to_string(), "windsurf".to_string()],
                    // GenericName=Text Editor lands here as a generic category alias.
                    name_aliases: Vec::new(),
                    generic_aliases: vec!["text editor".to_string()],
                    no_display: true,
                },
                AppManifest {
                    app_id: CanonicalAppId::from_registry("org.gnome.texteditor".to_string()),
                    display_name: "Text Editor".to_string(),
                    desktop_path: PathBuf::from(
                        "/usr/share/applications/org.gnome.TextEditor.desktop",
                    ),
                    exec_line: "gnome-text-editor %U".to_string(),
                    exec_fingerprint: sha256_bytes(b"gnome-text-editor %U"),
                    registered_schemes: Vec::new(),
                    name_aliases: Vec::new(),
                    generic_aliases: vec!["text editor".to_string()],
                    no_display: false,
                },
            ])
            .await;

        // Bare "editor" resolves via the TEXT_EDITOR class-alias list.
        let editor = registry
            .resolve_alias("editor")
            .expect("'editor' should resolve to the installed text editor");
        assert_eq!(
            editor.as_str(),
            "org.gnome.texteditor",
            "'editor' must map to the real editor, never the hidden URL handler"
        );
        // Explicit "text editor" must also resolve to the real editor.
        let text_editor = registry
            .resolve_alias("text editor")
            .expect("'text editor' should resolve to the installed text editor");
        assert_eq!(text_editor.as_str(), "org.gnome.texteditor");

        // The hidden handler is still launchable by its own ID and still owns its
        // URI scheme (the guard only protects human-facing class aliases).
        assert_eq!(
            registry
                .resolve_alias("devin-desktop-url-handler")
                .map(|id| id.as_str().to_string())
                .as_deref(),
            Some("devin-desktop-url-handler")
        );
        assert!(registry.registered_schemes().contains("devin"));
    }

    /// Issue #2 (the LIVE OpenApp bug): multiple **visible** heavyweight IDEs
    /// (`devin-desktop`, `code`, `kiro`, `vim` — all `NoDisplay=false`) advertise
    /// `GenericName=Text Editor`. With the guard ON they must NOT hijack the
    /// "text editor" / "editor" class alias away from the dedicated GNOME Text
    /// Editor, even when scanned LAST. The prior `no_display`-only guard did not
    /// cover this (these are visible apps), so "open the text editor" launched
    /// whichever IDE was scanned last and no-opped in ~20ms.
    #[tokio::test]
    async fn visible_ides_generic_name_does_not_hijack_text_editor_alias() {
        let mk = |id: &str, name: &str| AppManifest {
            app_id: CanonicalAppId::from_registry(id.to_string()),
            display_name: name.to_string(),
            desktop_path: PathBuf::from(format!("/usr/share/applications/{id}.desktop")),
            exec_line: format!("/usr/bin/{id} %F"),
            exec_fingerprint: sha256_bytes(id.as_bytes()),
            registered_schemes: Vec::new(),
            name_aliases: Vec::new(),
            // Every IDE shares the SAME generic category label.
            generic_aliases: vec!["text editor".to_string()],
            no_display: false,
        };
        let registry = Arc::new(InstalledAppRegistry {
            apps: Arc::new(RwLock::new(HashMap::new())),
            aliases: Arc::new(RwLock::new(HashMap::new())),
            schemes: Arc::new(RwLock::new(HashMap::new())),
        });
        // The real dedicated editor first, then IDEs scanned AFTER it.
        let manifests = vec![
            AppManifest {
                generic_aliases: vec!["text editor".to_string()],
                ..mk("org.gnome.texteditor", "Text Editor")
            },
            mk("devin-desktop", "Devin"),
            mk("code", "Visual Studio Code"),
            mk("kiro", "Kiro"),
        ];

        // Guard ON (default-on / desktop path): the built-in class alias +
        // dedicated editor win; the IDE category labels only fill unclaimed keys.
        registry
            .load_manifests_with_guard(manifests.clone(), true)
            .await;
        assert_eq!(
            registry
                .resolve_alias("text editor")
                .map(|i| i.as_str().to_string()),
            Some("org.gnome.texteditor".to_string()),
            "'text editor' must resolve to the dedicated editor, not a heavyweight IDE"
        );
        assert_eq!(
            registry
                .resolve_alias("editor")
                .map(|i| i.as_str().to_string()),
            Some("org.gnome.texteditor".to_string()),
            "bare 'editor' must resolve to the dedicated editor, not a heavyweight IDE"
        );

        // Guard OFF (rollback): prior byte-for-byte behavior — generic aliases
        // clobber, so the LAST-scanned visible IDE wins the shared category label.
        registry.load_manifests_with_guard(manifests, false).await;
        assert_eq!(
            registry
                .resolve_alias("text editor")
                .map(|i| i.as_str().to_string()),
            Some("kiro".to_string()),
            "guard OFF must restore the prior last-writer-wins clobber behavior"
        );
    }
}
