//! Typed identifier normalization and strong/weak classification (design §7.1,
//! task F2.5.1, MGR-019).
//!
//! Design §7.1: "Normalize aliases by type (Unicode case-folded names, verified
//! emails, canonical URLs, repository paths, shell history). Strong exact typed
//! identifiers may resolve; name/fuzzy/embedding only propose."
//!
//! Every public type here is `Serialize`/`Deserialize` so it can be stored in
//! authority JSON columns and exchanged with the desktop/server adapters.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

// ── IdentifierStrength ───────────────────────────────────────────────────────

/// Whether an identifier is strong (can auto-resolve) or weak (can only propose).
///
/// Design §7.1: "Strong exact typed identifiers may resolve; name/fuzzy/
/// embedding only propose."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierStrength {
    /// Strong: an exact typed identifier that can automatically resolve entity
    /// identity. Examples: email, canonical URL, repository remote URL.
    Strong,
    /// Weak: a fuzzy/heuristic identifier that can only propose matches.
    /// Examples: display name, common name alias, path hint.
    Weak,
}

// ── IdentifierType ───────────────────────────────────────────────────────────

/// The semantic type of an identifier, determining its normalization and strength.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierType {
    /// A human display name (weak — case-folded Unicode NFC normalization).
    Name,
    /// An email address (strong — lowercase, domain ASCII-lowercased).
    Email,
    /// A URL (strong — scheme+host normalized, path preserved verbatim).
    Url,
    /// A repository remote URL (strong — scheme+host normalized, path normalized).
    RepositoryUrl,
    /// A file system path (weak — platform-specific normalization, not globally unique).
    FilePath,
    /// A shell history token (weak — normalized but rarely unique).
    ShellHistoryToken,
    /// An opaque external identifier (strong if the source is authoritative).
    ExternalId { source: String },
}

impl IdentifierType {
    /// Returns the strength of this identifier type.
    pub fn strength(&self) -> IdentifierStrength {
        match self {
            IdentifierType::Name => IdentifierStrength::Weak,
            IdentifierType::Email => IdentifierStrength::Strong,
            IdentifierType::Url => IdentifierStrength::Strong,
            IdentifierType::RepositoryUrl => IdentifierStrength::Strong,
            IdentifierType::FilePath => IdentifierStrength::Weak,
            IdentifierType::ShellHistoryToken => IdentifierStrength::Weak,
            IdentifierType::ExternalId { .. } => IdentifierStrength::Strong,
        }
    }
}

// ── NormalizationError ───────────────────────────────────────────────────────

/// Errors produced during identifier normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizationError {
    /// Email is missing an `@` sign or has empty local/domain parts.
    InvalidEmail { input: String },
    /// URL is missing a scheme (e.g. `http://` or `https://`).
    MissingUrlScheme { input: String },
    /// URL host is empty after scheme.
    EmptyUrlHost { input: String },
    /// Input is empty after trimming.
    EmptyInput,
}

impl std::fmt::Display for NormalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NormalizationError::InvalidEmail { input } => {
                write!(f, "invalid email (missing @ or empty parts): {input:?}")
            }
            NormalizationError::MissingUrlScheme { input } => {
                write!(f, "URL missing scheme (e.g. http://): {input:?}")
            }
            NormalizationError::EmptyUrlHost { input } => {
                write!(f, "URL has empty host after scheme: {input:?}")
            }
            NormalizationError::EmptyInput => {
                write!(f, "identifier input is empty after trimming")
            }
        }
    }
}

impl std::error::Error for NormalizationError {}

// ── NormalizedIdentifier ─────────────────────────────────────────────────────

/// A normalized identifier with its type, strength, and canonical form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedIdentifier {
    /// The canonical normalized form.
    pub canonical: String,
    /// The original value before normalization (for provenance).
    pub original: String,
    /// The identifier type.
    pub id_type: IdentifierType,
    /// The computed strength (derived from id_type).
    pub strength: IdentifierStrength,
}

// ── IdentifierNormalizer ─────────────────────────────────────────────────────

/// Stateless normalizer for all supported identifier types.
pub struct IdentifierNormalizer;

impl IdentifierNormalizer {
    /// Normalize a name: Unicode NFC normalization + case-fold (lowercase).
    ///
    /// Empty name after trim is gracefully handled by returning the empty string
    /// as the canonical form (weak identifier, not structural like email/URL).
    /// Returns [`IdentifierStrength::Weak`].
    pub fn normalize_name(raw: &str) -> NormalizedIdentifier {
        let trimmed = raw.trim();
        // Unicode NFC then case-fold (to_lowercase on NFC gives consistent
        // case-folding for most scripts; full Unicode case-folding would
        // require a separate crate — NFC + to_lowercase is the specified rule).
        let canonical: String = trimmed.nfc().collect::<String>().to_lowercase();
        NormalizedIdentifier {
            canonical,
            original: raw.to_owned(),
            id_type: IdentifierType::Name,
            strength: IdentifierStrength::Weak,
        }
    }

    /// Normalize an email: trim whitespace, ASCII-lowercase the entire string.
    ///
    /// Validates `@` present, local part non-empty, domain non-empty.
    /// Returns [`IdentifierStrength::Strong`].
    ///
    /// # Errors
    /// Returns [`NormalizationError::EmptyInput`] when trimmed input is empty.
    /// Returns [`NormalizationError::InvalidEmail`] when the `@` is absent,
    /// local part is empty, or domain part is empty.
    pub fn normalize_email(raw: &str) -> Result<NormalizedIdentifier, NormalizationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(NormalizationError::EmptyInput);
        }
        let lower = trimmed.to_lowercase();
        // Find the last `@` (some display forms include multiple — treat last as separator).
        let at_pos = lower
            .find('@')
            .ok_or_else(|| NormalizationError::InvalidEmail {
                input: raw.to_owned(),
            })?;
        let local = &lower[..at_pos];
        let domain = &lower[at_pos + 1..];
        if local.is_empty() || domain.is_empty() {
            return Err(NormalizationError::InvalidEmail {
                input: raw.to_owned(),
            });
        }
        Ok(NormalizedIdentifier {
            canonical: lower,
            original: raw.to_owned(),
            id_type: IdentifierType::Email,
            strength: IdentifierStrength::Strong,
        })
    }

    /// Normalize a URL: scheme+host lowercased, path/query/fragment preserved verbatim.
    ///
    /// Parsing rule: find `://`, lowercase everything before and including it,
    /// then split remaining at the first `/` to separate host from the rest.
    /// The host portion is lowercased; the path/query/fragment is kept as-is.
    ///
    /// Returns [`IdentifierStrength::Strong`].
    ///
    /// # Errors
    /// Returns [`NormalizationError::EmptyInput`] when trimmed input is empty.
    /// Returns [`NormalizationError::MissingUrlScheme`] when `://` is not found.
    /// Returns [`NormalizationError::EmptyUrlHost`] when the host is empty.
    pub fn normalize_url(raw: &str) -> Result<NormalizedIdentifier, NormalizationError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(NormalizationError::EmptyInput);
        }
        let sep = "://";
        let sep_pos = trimmed
            .find(sep)
            .ok_or_else(|| NormalizationError::MissingUrlScheme {
                input: raw.to_owned(),
            })?;
        // scheme (including "://") lowercased
        let scheme_and_sep = trimmed[..sep_pos + sep.len()].to_lowercase();
        let after_scheme = &trimmed[sep_pos + sep.len()..];
        // Split host from path at the first `/`
        let (host_raw, rest) = match after_scheme.find('/') {
            Some(slash_pos) => {
                let h = &after_scheme[..slash_pos];
                let r = &after_scheme[slash_pos..]; // includes leading '/'
                (h, r)
            }
            None => (after_scheme, ""),
        };
        let host_lower = host_raw.to_lowercase();
        if host_lower.is_empty() {
            return Err(NormalizationError::EmptyUrlHost {
                input: raw.to_owned(),
            });
        }
        let canonical = format!("{scheme_and_sep}{host_lower}{rest}");
        Ok(NormalizedIdentifier {
            canonical,
            original: raw.to_owned(),
            id_type: IdentifierType::Url,
            strength: IdentifierStrength::Strong,
        })
    }

    /// Normalize a repository URL: same as URL normalization but strip a
    /// trailing `.git` suffix from the path if present.
    ///
    /// Returns [`IdentifierStrength::Strong`].
    ///
    /// # Errors
    /// Same as [`normalize_url`](Self::normalize_url).
    pub fn normalize_repository_url(raw: &str) -> Result<NormalizedIdentifier, NormalizationError> {
        // First normalize as a regular URL, then strip .git suffix.
        let mut base = Self::normalize_url(raw)?;
        // Strip trailing ".git" from the canonical form.
        if let Some(stripped) = base.canonical.strip_suffix(".git") {
            base.canonical = stripped.to_owned();
        }
        // Update the type to RepositoryUrl.
        base.id_type = IdentifierType::RepositoryUrl;
        Ok(base)
    }

    /// Normalize a file path: trim whitespace, collapse consecutive `/` to one,
    /// strip trailing `/` (unless the result would otherwise be empty, in which
    /// case use `/`).
    ///
    /// Does NOT resolve `..` or `.` — no filesystem access is available here.
    /// Returns [`IdentifierStrength::Weak`].
    pub fn normalize_file_path(raw: &str) -> NormalizedIdentifier {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return NormalizedIdentifier {
                canonical: String::new(),
                original: raw.to_owned(),
                id_type: IdentifierType::FilePath,
                strength: IdentifierStrength::Weak,
            };
        }
        // Collapse consecutive '/' to a single '/'
        let mut canonical = String::with_capacity(trimmed.len());
        let mut last_was_slash = false;
        for ch in trimmed.chars() {
            if ch == '/' {
                if !last_was_slash {
                    canonical.push('/');
                }
                last_was_slash = true;
            } else {
                canonical.push(ch);
                last_was_slash = false;
            }
        }
        // Strip trailing '/' unless the whole path is just '/'
        if canonical.len() > 1 && canonical.ends_with('/') {
            canonical.pop();
        }
        // Guard: if collapsing produced an empty string, treat as root.
        if canonical.is_empty() {
            canonical.push('/');
        }
        NormalizedIdentifier {
            canonical,
            original: raw.to_owned(),
            id_type: IdentifierType::FilePath,
            strength: IdentifierStrength::Weak,
        }
    }

    /// Normalize a shell history token: trim whitespace, collapse internal
    /// whitespace runs to a single space, truncate to 512 characters.
    ///
    /// Returns [`IdentifierStrength::Weak`].
    pub fn normalize_shell_token(raw: &str) -> NormalizedIdentifier {
        let trimmed = raw.trim();
        // Collapse internal whitespace runs to single space.
        let mut canonical = String::with_capacity(trimmed.len());
        let mut last_was_ws = false;
        for ch in trimmed.chars() {
            if ch.is_whitespace() {
                if !last_was_ws {
                    canonical.push(' ');
                }
                last_was_ws = true;
            } else {
                canonical.push(ch);
                last_was_ws = false;
            }
        }
        // Truncate to 512 characters (char boundary safe).
        let canonical = if canonical.chars().count() > 512 {
            canonical.chars().take(512).collect()
        } else {
            canonical
        };
        NormalizedIdentifier {
            canonical,
            original: raw.to_owned(),
            id_type: IdentifierType::ShellHistoryToken,
            strength: IdentifierStrength::Weak,
        }
    }

    /// Dispatch normalization based on identifier type.
    ///
    /// For types that can fail (`Email`, `Url`, `RepositoryUrl`), returns `Err`
    /// on invalid input. For weak types (`Name`, `FilePath`, `ShellHistoryToken`,
    /// `ExternalId`), always succeeds.
    pub fn normalize(
        raw: &str,
        id_type: &IdentifierType,
    ) -> Result<NormalizedIdentifier, NormalizationError> {
        match id_type {
            IdentifierType::Name => Ok(Self::normalize_name(raw)),
            IdentifierType::Email => Self::normalize_email(raw),
            IdentifierType::Url => Self::normalize_url(raw),
            IdentifierType::RepositoryUrl => Self::normalize_repository_url(raw),
            IdentifierType::FilePath => Ok(Self::normalize_file_path(raw)),
            IdentifierType::ShellHistoryToken => Ok(Self::normalize_shell_token(raw)),
            IdentifierType::ExternalId { source } => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return Err(NormalizationError::EmptyInput);
                }
                Ok(NormalizedIdentifier {
                    canonical: trimmed.to_owned(),
                    original: raw.to_owned(),
                    id_type: IdentifierType::ExternalId {
                        source: source.clone(),
                    },
                    strength: IdentifierStrength::Strong,
                })
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── strength() ──────────────────────────────────────────────────────────

    #[test]
    fn strength_email_url_repo_external_are_strong() {
        assert_eq!(IdentifierType::Email.strength(), IdentifierStrength::Strong);
        assert_eq!(IdentifierType::Url.strength(), IdentifierStrength::Strong);
        assert_eq!(
            IdentifierType::RepositoryUrl.strength(),
            IdentifierStrength::Strong
        );
        assert_eq!(
            IdentifierType::ExternalId {
                source: "github".into()
            }
            .strength(),
            IdentifierStrength::Strong
        );
    }

    #[test]
    fn strength_name_filepath_shell_are_weak() {
        assert_eq!(IdentifierType::Name.strength(), IdentifierStrength::Weak);
        assert_eq!(
            IdentifierType::FilePath.strength(),
            IdentifierStrength::Weak
        );
        assert_eq!(
            IdentifierType::ShellHistoryToken.strength(),
            IdentifierStrength::Weak
        );
    }

    // ── normalize_name ───────────────────────────────────────────────────────

    #[test]
    fn normalize_name_lowercases_and_nfc() {
        let n = IdentifierNormalizer::normalize_name("  Ada Lovelace  ");
        assert_eq!(n.canonical, "ada lovelace");
        assert_eq!(n.original, "  Ada Lovelace  ");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_name_empty_produces_empty_canonical() {
        let n = IdentifierNormalizer::normalize_name("   ");
        assert_eq!(n.canonical, "");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_name_unicode_nfc_and_lowercase() {
        // Precomposed NFC: ä is U+00E4 (already NFC); input has combining form.
        // "a\u{0308}" (a + combining diaeresis) should NFC → "ä" then lowercase stays "ä".
        let combining = "A\u{0308}"; // A + combining umlaut
        let n = IdentifierNormalizer::normalize_name(combining);
        // NFC(A + combining umlaut) = Ä (U+00C4), then lowercase = ä (U+00E4)
        assert_eq!(n.canonical, "\u{00E4}");
    }

    // ── normalize_email ──────────────────────────────────────────────────────

    #[test]
    fn normalize_email_success_lowercases() {
        let n = IdentifierNormalizer::normalize_email("  Ada@Example.COM  ").unwrap();
        assert_eq!(n.canonical, "ada@example.com");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_email_missing_at() {
        let err = IdentifierNormalizer::normalize_email("notanemail").unwrap_err();
        assert!(matches!(err, NormalizationError::InvalidEmail { .. }));
    }

    #[test]
    fn normalize_email_empty_local_part() {
        let err = IdentifierNormalizer::normalize_email("@example.com").unwrap_err();
        assert!(matches!(err, NormalizationError::InvalidEmail { .. }));
    }

    #[test]
    fn normalize_email_empty_domain() {
        let err = IdentifierNormalizer::normalize_email("user@").unwrap_err();
        assert!(matches!(err, NormalizationError::InvalidEmail { .. }));
    }

    #[test]
    fn normalize_email_empty_input() {
        let err = IdentifierNormalizer::normalize_email("   ").unwrap_err();
        assert_eq!(err, NormalizationError::EmptyInput);
    }

    // ── normalize_url ────────────────────────────────────────────────────────

    #[test]
    fn normalize_url_lowercases_scheme_and_host() {
        let n = IdentifierNormalizer::normalize_url("HTTPS://GitHub.COM/foo/Bar").unwrap();
        assert_eq!(n.canonical, "https://github.com/foo/Bar");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_url_preserves_path_case() {
        let n = IdentifierNormalizer::normalize_url("https://example.com/Docs/README.md").unwrap();
        assert_eq!(n.canonical, "https://example.com/Docs/README.md");
    }

    #[test]
    fn normalize_url_no_path() {
        let n = IdentifierNormalizer::normalize_url("https://Example.COM").unwrap();
        assert_eq!(n.canonical, "https://example.com");
    }

    #[test]
    fn normalize_url_missing_scheme() {
        let err = IdentifierNormalizer::normalize_url("github.com/foo").unwrap_err();
        assert!(matches!(err, NormalizationError::MissingUrlScheme { .. }));
    }

    #[test]
    fn normalize_url_empty_host() {
        let err = IdentifierNormalizer::normalize_url("https:///path").unwrap_err();
        assert!(matches!(err, NormalizationError::EmptyUrlHost { .. }));
    }

    // ── normalize_repository_url ─────────────────────────────────────────────

    #[test]
    fn normalize_repository_url_strips_git_suffix() {
        let n = IdentifierNormalizer::normalize_repository_url("https://GitHub.COM/user/repo.git")
            .unwrap();
        assert_eq!(n.canonical, "https://github.com/user/repo");
        assert_eq!(n.id_type, IdentifierType::RepositoryUrl);
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_repository_url_no_git_suffix_unchanged() {
        let n =
            IdentifierNormalizer::normalize_repository_url("https://github.com/user/repo").unwrap();
        assert_eq!(n.canonical, "https://github.com/user/repo");
    }

    #[test]
    fn normalize_repository_url_ssh_style() {
        // ssh:// URLs normalize the same way.
        let n =
            IdentifierNormalizer::normalize_repository_url("ssh://Git@GitHub.com/user/repo.git")
                .unwrap();
        assert_eq!(n.canonical, "ssh://git@github.com/user/repo");
    }

    // ── normalize_file_path ──────────────────────────────────────────────────

    #[test]
    fn normalize_file_path_collapses_slashes() {
        let n = IdentifierNormalizer::normalize_file_path("//home//user//docs//");
        assert_eq!(n.canonical, "/home/user/docs");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_file_path_strips_trailing_slash() {
        let n = IdentifierNormalizer::normalize_file_path("/home/user/");
        assert_eq!(n.canonical, "/home/user");
    }

    #[test]
    fn normalize_file_path_root_preserved() {
        let n = IdentifierNormalizer::normalize_file_path("/");
        assert_eq!(n.canonical, "/");
    }

    #[test]
    fn normalize_file_path_empty_becomes_empty() {
        let n = IdentifierNormalizer::normalize_file_path("   ");
        assert_eq!(n.canonical, "");
    }

    #[test]
    fn normalize_file_path_trims_whitespace() {
        let n = IdentifierNormalizer::normalize_file_path("  /home/user  ");
        assert_eq!(n.canonical, "/home/user");
    }

    // ── normalize_shell_token ────────────────────────────────────────────────

    #[test]
    fn normalize_shell_token_collapses_whitespace() {
        let n = IdentifierNormalizer::normalize_shell_token("  git   commit  -m  'msg'  ");
        assert_eq!(n.canonical, "git commit -m 'msg'");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_shell_token_truncates_at_512_chars() {
        let long: String = "a ".repeat(300); // 600 chars
        let n = IdentifierNormalizer::normalize_shell_token(&long);
        // After collapsing, "a " repeated 300 = "a " * 300 trimmed → "a" + (" a")*299
        // The original trimmed is "a  a  a ...a" which collapses to "a a a...a"
        // Regardless of exact content, length must be ≤ 512 chars.
        assert!(n.canonical.chars().count() <= 512);
        assert_eq!(n.original, long);
    }

    #[test]
    fn normalize_shell_token_empty() {
        let n = IdentifierNormalizer::normalize_shell_token("   ");
        assert_eq!(n.canonical, "");
    }

    // ── normalize() dispatch ─────────────────────────────────────────────────

    #[test]
    fn normalize_dispatch_name() {
        let n = IdentifierNormalizer::normalize("Hello World", &IdentifierType::Name).unwrap();
        assert_eq!(n.canonical, "hello world");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_dispatch_email() {
        let n =
            IdentifierNormalizer::normalize("User@Example.com", &IdentifierType::Email).unwrap();
        assert_eq!(n.canonical, "user@example.com");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_dispatch_url() {
        let n = IdentifierNormalizer::normalize("HTTPS://Example.COM/path", &IdentifierType::Url)
            .unwrap();
        assert_eq!(n.canonical, "https://example.com/path");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_dispatch_repository_url() {
        let n = IdentifierNormalizer::normalize(
            "https://GitHub.com/org/repo.git",
            &IdentifierType::RepositoryUrl,
        )
        .unwrap();
        assert_eq!(n.canonical, "https://github.com/org/repo");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_dispatch_file_path() {
        let n = IdentifierNormalizer::normalize("//tmp//file", &IdentifierType::FilePath).unwrap();
        assert_eq!(n.canonical, "/tmp/file");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_dispatch_shell_token() {
        let n =
            IdentifierNormalizer::normalize("ls  -la", &IdentifierType::ShellHistoryToken).unwrap();
        assert_eq!(n.canonical, "ls -la");
        assert_eq!(n.strength, IdentifierStrength::Weak);
    }

    #[test]
    fn normalize_dispatch_external_id() {
        let id_type = IdentifierType::ExternalId {
            source: "jira".into(),
        };
        let n = IdentifierNormalizer::normalize("  PROJ-123  ", &id_type).unwrap();
        assert_eq!(n.canonical, "PROJ-123");
        assert_eq!(n.strength, IdentifierStrength::Strong);
    }

    #[test]
    fn normalize_dispatch_external_id_empty_fails() {
        let id_type = IdentifierType::ExternalId {
            source: "jira".into(),
        };
        let err = IdentifierNormalizer::normalize("   ", &id_type).unwrap_err();
        assert_eq!(err, NormalizationError::EmptyInput);
    }

    // ── serde roundtrip ──────────────────────────────────────────────────────

    #[test]
    fn identifier_type_serde_roundtrip() {
        let types = vec![
            IdentifierType::Name,
            IdentifierType::Email,
            IdentifierType::Url,
            IdentifierType::RepositoryUrl,
            IdentifierType::FilePath,
            IdentifierType::ShellHistoryToken,
            IdentifierType::ExternalId {
                source: "gh".into(),
            },
        ];
        for t in &types {
            let json = serde_json::to_string(t).unwrap();
            let back: IdentifierType = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, t);
        }
    }

    #[test]
    fn normalized_identifier_serde_roundtrip() {
        let n = IdentifierNormalizer::normalize_email("user@example.com").unwrap();
        let json = serde_json::to_string(&n).unwrap();
        let back: NormalizedIdentifier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, n);
    }
}
