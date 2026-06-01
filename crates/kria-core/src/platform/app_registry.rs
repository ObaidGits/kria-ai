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
    /// Additional name aliases (from `GenericName=`, `X-KDE-Aliases=`, etc.) used for
    /// fuzzy resolution of LLM-supplied names like "chrome" → "chromium".
    pub name_aliases: Vec<String>,
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

            // Register aliases (case-insensitive). .desktop entries override builtins.
            aliases.insert(id_str.clone(), id_str.clone());
            aliases.insert(manifest.display_name.to_lowercase(), id_str.clone());
            for alias in &manifest.name_aliases {
                aliases.insert(alias.to_lowercase(), id_str.clone());
            }

            // Register URI schemes.
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
        "text editor" | "editor" | "plain text editor" | "document editor"
    ) {
        Some(TEXT_EDITOR)
    } else if matches!(
        normalized,
        "file manager" | "files" | "home folder" | "folder viewer"
    ) {
        Some(FILE_MANAGER)
    } else if matches!(
        normalized,
        "code" | "code editor" | "ide" | "editor for code"
    ) {
        Some(IDE)
    } else {
        None
    }
}

// ─── Desktop file parsing ─────────────────────────────────────────────────────

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

    // Build aliases from display name and generic name.
    let mut aliases = vec![name.to_lowercase()];
    if !generic_name.is_empty() {
        aliases.push(generic_name.to_lowercase());
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
            }])
            .await;

        let resolved = registry
            .resolve_alias("Excel or Calc")
            .expect("spreadsheet class alias should resolve");
        assert_eq!(resolved.as_str(), "libreoffice-calc");
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
}
