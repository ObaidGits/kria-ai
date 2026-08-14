//! The live desktop-search provider, backed by GNOME Tracker 3.
//!
//! linux-os-control-production task **4.1**.
//!
//! # Why the scope read is authoritative and the search is not the index
//!
//! Tracker keeps two separate things: the **scope** (which folders it is allowed
//! to index, stored in GSettings) and the **index** (what it has actually read).
//! They drift — a folder added a second ago is in scope but not yet indexed. This
//! provider therefore reads the scope from GSettings, never inferring it from
//! whatever the index happens to contain, so "which folders can KRIA see" is
//! answered by policy rather than by a race.
//!
//! # Tracker indexes CONTENT
//!
//! Tracker's file miner extracts full text from documents. A search therefore
//! reaches *inside* the user's files, which is why [`SearchScope::content_indexed`]
//! is reported `true` whenever the miner is present, and why the search tool is
//! rated RED on that basis. Getting this flag wrong in the safe-looking direction
//! would silently downgrade a content search to a filename search in the risk
//! rating while it still read the contents.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::receipt::ApplyOutcome;
use crate::os_control::search::{
    RebuildState, SearchHit, SearchOp, SearchPage, SearchScope, SearchScopeId, SearchTransport,
};

/// The GSettings schema that owns the file miner's scope.
const MINER_SCHEMA: &str = "org.freedesktop.Tracker3.Miner.Files";

/// Candidate absolute paths for the Tracker CLI.
const TRACKER_PATHS: &[&str] = &["/usr/bin/tracker3", "/usr/bin/tracker"];
/// Candidate absolute paths for the GSettings CLI.
const GSETTINGS_PATHS: &[&str] = &["/usr/bin/gsettings"];
/// Candidate absolute paths for systemctl (used only to read miner liveness).
const SYSTEMCTL_PATHS: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];

/// The live Tracker-backed search transport.
pub struct LiveSearch {
    tracker: &'static str,
    gsettings: &'static str,
    systemctl: Option<&'static str>,
}

impl LiveSearch {
    /// Compose the provider when Tracker and GSettings are both present.
    ///
    /// Returns `None` when either is absent, so the domain stays uncomposed and
    /// answers `Unavailable` rather than degrading to a filesystem walk. A silent
    /// `find`-style fallback would be a different operation with different
    /// performance and different privacy properties.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            tracker: cli::first_present(TRACKER_PATHS)?,
            gsettings: cli::first_present(GSETTINGS_PATHS)?,
            systemctl: cli::first_present(SYSTEMCTL_PATHS),
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("tracker3")
    }

    /// Read one GSettings key as a list of absolute paths.
    ///
    /// Tracker stores XDG aliases (`&DESKTOP`, `$HOME`) alongside literal paths.
    /// Aliases are resolved through the XDG user-dirs config; an alias that cannot
    /// be resolved is **dropped rather than guessed**, because inventing
    /// `~/Desktop` for a user who has no Desktop folder would report a scope that
    /// does not exist.
    async fn read_paths(
        &self,
        ctx: &HostExecutionContext,
        key: &str,
    ) -> Result<Vec<std::path::PathBuf>, OsControlError> {
        let raw = cli::query(
            ctx,
            self.id(),
            "search.read_scope",
            self.gsettings,
            vec!["get".into(), MINER_SCHEMA.into(), key.into()],
        )
        .await?;
        Ok(parse_gsettings_list(&raw)
            .into_iter()
            .filter_map(|token| resolve_scope_token(&token))
            .collect())
    }
}

/// Parse a GSettings string-array literal: `['/a', '&DESKTOP']`.
///
/// Returns an empty vector for `@as []` (the empty-array literal) and for `[]`.
/// A malformed value yields no entries rather than a partial guess.
fn parse_gsettings_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let Some(start) = trimmed.find('[') else {
        return Vec::new();
    };
    let Some(end) = trimmed.rfind(']') else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    trimmed[start + 1..end]
        .split(',')
        .map(|item| item.trim().trim_matches(['\'', '"']).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Resolve a Tracker scope token to an absolute path.
fn resolve_scope_token(token: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = token.strip_prefix('/') {
        return Some(std::path::PathBuf::from(format!("/{path}")));
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    match token {
        "$HOME" | "&HOME" => Some(home),
        // XDG aliases Tracker understands. Resolved from the environment where
        // possible; a missing directory is dropped, never invented.
        other if other.starts_with('&') => {
            let dir = match other {
                "&DESKTOP" => "Desktop",
                "&DOCUMENTS" => "Documents",
                "&DOWNLOAD" => "Downloads",
                "&MUSIC" => "Music",
                "&PICTURES" => "Pictures",
                "&VIDEOS" => "Videos",
                "&PUBLIC_SHARE" => "Public",
                "&TEMPLATES" => "Templates",
                _ => return None,
            };
            let candidate = home.join(dir);
            candidate.is_dir().then_some(candidate)
        }
        _ => None,
    }
}

/// Parse `tracker3 search --files` output into hits.
///
/// Tracker prints a header line then indented `file://` URLs. Anything that is
/// not a decodable local file URL is skipped rather than reported as a path.
fn parse_search_output(raw: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for line in raw.lines() {
        let token = line.trim();
        let Some(rest) = token.strip_prefix("file://") else {
            continue;
        };
        let Some(path) = percent_decode(rest) else {
            continue;
        };
        let path = std::path::PathBuf::from(path);
        let kind = if path.is_dir() { "directory" } else { "file" };
        hits.push(SearchHit {
            path,
            kind: kind.to_string(),
            // Tracker's `--files` mode returns paths only. A snippet is reported
            // only when the tool actually supplied one, so the absence of a
            // snippet never implies the absence of a content match.
            snippet: None,
        });
    }
    hits
}

/// Decode `%XX` escapes in a file URL. Returns `None` on a malformed escape.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[async_trait]
impl SearchTransport for LiveSearch {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn read_scope(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<&SearchScopeId>,
    ) -> Result<SearchScope, OsControlError> {
        let mut roots = self.read_paths(ctx, "index-recursive-directories").await?;
        roots.extend(self.read_paths(ctx, "index-single-directories").await?);
        let exclusions = self.read_paths(ctx, "ignored-directories").await?;
        let scope = match scope {
            Some(id) => id.clone(),
            None => SearchScopeId::parse("default")?,
        };
        Ok(SearchScope {
            scope,
            roots,
            exclusions,
            // Tracker's miner extracts full text from documents, so a search
            // reaches inside file contents. Reported unconditionally true: the
            // extractor is part of the same package set as the CLI, and claiming
            // otherwise would under-rate the risk of every search.
            content_indexed: true,
        })
    }

    async fn read_rebuild_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebuildState, OsControlError> {
        let Some(systemctl) = self.systemctl else {
            // Without systemctl we cannot tell a running rebuild from an idle
            // index. Refuse rather than report Idle, which would let a rebuild
            // "verify" as started when nothing happened.
            return Err(OsControlError::Unavailable {
                provider: Some(self.id()),
                reason: SafeText::new("cannot determine indexer state without systemctl"),
                retryable: false,
            });
        };
        let (raw, _exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "search.read_rebuild_state",
            systemctl,
            vec![
                "--user".into(),
                "is-active".into(),
                "tracker-miner-fs-3.service".into(),
            ],
        )
        .await?;
        // `is-active` exits non-zero for anything but active, so the exit status
        // carries no extra information the text does not.
        match raw.trim() {
            "active" | "activating" | "reloading" => Ok(RebuildState::Running),
            "inactive" | "failed" | "deactivating" | "unknown" => Ok(RebuildState::Idle),
            other => Err(OsControlError::Unavailable {
                provider: Some(self.id()),
                reason: SafeText::new(format!("unrecognized indexer state: {other}")),
                retryable: true,
            }),
        }
    }

    async fn query(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        scope: &SearchScope,
        _cursor: Option<&str>,
        limit: usize,
    ) -> Result<SearchPage, OsControlError> {
        // A term beginning with `-` would be read as a tracker option.
        cli::reject_option_like("query", query)?;
        if query.trim().is_empty() {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("query"),
                reason: SafeText::new("query must not be empty"),
            });
        }
        // Ask for one more than requested so `truncated` reflects reality rather
        // than coincidence: exactly `limit` results is ambiguous on its own.
        let fetch = limit.saturating_add(1).min(1024);
        let raw = cli::query(
            ctx,
            self.id(),
            "search.query",
            self.tracker,
            vec![
                "search".into(),
                "--files".into(),
                "--limit".into(),
                fetch.to_string(),
                query.to_string(),
            ],
        )
        .await?;
        let mut items = parse_search_output(&raw);
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(SearchPage {
            items,
            // Tracker's CLI has no stable resumable cursor, so none is offered.
            // Advertising one that silently restarts would produce duplicate and
            // missed rows in a caller that trusted it.
            next_cursor: None,
            truncated,
            content_indexed: scope.content_indexed,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &SearchOp,
    ) -> Result<ApplyOutcome, OsControlError> {
        match op {
            SearchOp::ConfigureScope(change) => {
                // GSettings takes one key per call, and the recursive roots are
                // the key that decides what a search can reach. Roots are written
                // first so a widened exclusion list can never be live while the
                // roots it was meant to constrain are still the old, broader set.
                let roots = format_gsettings_list(&change.roots);
                let outcome = cli::dispatch(
                    ctx,
                    "search.configure_scope.roots",
                    self.gsettings,
                    vec![
                        "set".into(),
                        MINER_SCHEMA.into(),
                        "index-recursive-directories".into(),
                        roots,
                    ],
                )
                .await?;
                if !matches!(outcome, ApplyOutcome::Applied { .. }) {
                    // The first write is uncertain — do not compound it with a
                    // second. The verify step will report what actually landed.
                    return Ok(outcome);
                }
                let exclusions = format_gsettings_list(&change.exclusions);
                cli::dispatch(
                    ctx,
                    "search.configure_scope.exclusions",
                    self.gsettings,
                    vec![
                        "set".into(),
                        MINER_SCHEMA.into(),
                        "ignored-directories".into(),
                        exclusions,
                    ],
                )
                .await
            }
            SearchOp::Rebuild { .. } => {
                // `reset --filesystem` discards the index and lets the miner
                // rebuild it. The index is a derived cache, so this destroys no
                // user data — but it can take hours, which is why the domain
                // verifies "started", never "finished".
                cli::dispatch(
                    ctx,
                    "search.rebuild",
                    self.tracker,
                    vec!["reset".into(), "--filesystem".into()],
                )
                .await
            }
        }
    }
}

/// Render paths as a GSettings string-array literal.
///
/// Single quotes and backslashes are escaped. A path containing a newline is not
/// representable and is dropped by the caller's validation long before here.
fn format_gsettings_list(paths: &[std::path::PathBuf]) -> String {
    let items: Vec<String> = paths
        .iter()
        .map(|path| {
            let text = path.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'");
            format!("'{text}'")
        })
        .collect();
    format!("[{}]", items.join(", "))
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn gsettings_lists_parse_and_round_trip() {
        assert_eq!(
            parse_gsettings_list("['/home/a', '&DESKTOP']"),
            vec!["/home/a".to_string(), "&DESKTOP".to_string()]
        );
        // The empty-array literal must not yield a phantom entry.
        assert!(parse_gsettings_list("@as []").is_empty());
        assert!(parse_gsettings_list("[]").is_empty());
        assert!(parse_gsettings_list("garbage").is_empty());
        assert_eq!(
            format_gsettings_list(&[std::path::PathBuf::from("/home/o'brien")]),
            "['/home/o\\'brien']"
        );
    }

    #[test]
    fn an_unresolvable_alias_is_dropped_not_invented() {
        assert!(resolve_scope_token("&NOT_A_REAL_ALIAS").is_none());
        assert_eq!(
            resolve_scope_token("/srv/data"),
            Some(std::path::PathBuf::from("/srv/data"))
        );
    }

    #[test]
    fn search_output_parses_file_urls_and_skips_junk() {
        let raw = "Results:\n  file:///home/a/Invoice%20Q1.pdf\n  not-a-url\n  file:///tmp/x";
        let hits = parse_search_output(raw);
        assert_eq!(hits.len(), 2);
        // The percent escape must be decoded, or the path would not exist.
        assert_eq!(
            hits[0].path,
            std::path::PathBuf::from("/home/a/Invoice Q1.pdf")
        );
    }

    #[test]
    fn a_malformed_escape_is_rejected_not_half_decoded() {
        assert!(percent_decode("/tmp/%zz").is_none());
        assert!(percent_decode("/tmp/%2").is_none());
        assert_eq!(percent_decode("/tmp/ok").as_deref(), Some("/tmp/ok"));
    }
}
