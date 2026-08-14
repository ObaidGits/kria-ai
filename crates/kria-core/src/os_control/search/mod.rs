//! Desktop search: querying the index, reading and configuring its scope, and
//! rebuilding it.
//!
//! linux-os-control-production task **4.1** (OSC-024).
//!
//! # Why `search_desktop`'s risk is conditional
//!
//! The frozen contract rates it **RED when the resolved scope is content-indexed**
//! and GREEN otherwise. That distinction is the whole point: a filename search
//! reveals names, while a *content* search reads inside the user's documents and
//! can surface a sentence from a private file in a result snippet. Same tool, two
//! very different exposures, so the risk follows the scope rather than the verb.
//!
//! # Configuring scope is RED for a reason that is easy to miss
//!
//! `configure_search_scope` does not read anything by itself — it decides what
//! *will* be indexed from now on. Adding a root silently widens what every future
//! search can reach, including content. So the roots are validated, bounded, and
//! the operation is RED even though nothing is read at the time it runs.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;
use std::path::PathBuf;

/// The provider identity.
pub const SEARCH_PROVIDER_ID: &str = "search-tracker";

/// Largest page a search may return.
pub const SEARCH_PAGE_MAX: usize = 256;

/// Default page size.
pub const SEARCH_PAGE_DEFAULT: usize = 25;

/// Most roots a scope may contain (frozen contract bound).
pub const MAX_SCOPE_ROOTS: usize = 256;

/// Most exclusions a scope may contain.
pub const MAX_SCOPE_EXCLUSIONS: usize = 64;

/// Longest query text accepted.
pub const MAX_QUERY_CHARS: usize = 512;

/// A search scope's stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SearchScopeId(String);

impl SearchScopeId {
    /// Validate and wrap a scope id.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
        let raw = raw.as_ref().trim();
        let ok = !raw.is_empty()
            && raw.len() <= 128
            && !raw.starts_with('-')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !ok {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("scope"),
                reason: SafeText::new("scope must be a stable scope id"),
            });
        }
        Ok(Self(raw.to_string()))
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The resolved facts about one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchScope {
    /// The scope's id.
    pub scope: SearchScopeId,
    /// Indexed roots.
    pub roots: Vec<PathBuf>,
    /// Excluded paths.
    pub exclusions: Vec<PathBuf>,
    /// Whether file **contents** are indexed, not just names.
    ///
    /// This single flag is what turns `search_desktop` from GREEN to RED, so it is
    /// never inferred — it comes from the index's own configuration.
    pub content_indexed: bool,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The matching path.
    pub path: PathBuf,
    /// The item's kind as the index reports it.
    pub kind: String,
    /// A bounded snippet, present **only** for a content-indexed scope.
    ///
    /// `None` for a name-only scope: there is no content to quote, and inventing
    /// one would leak more than the search was rated for.
    pub snippet: Option<SafeText>,
}

/// One page of hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    /// The hits on this page.
    pub items: Vec<SearchHit>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether the result set was cut short.
    pub truncated: bool,
    /// Whether the scope searched indexes content.
    pub content_indexed: bool,
}

/// The state of an index rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildState {
    /// No rebuild is running.
    Idle,
    /// A rebuild is in progress.
    Running,
}

/// Which fact an observation carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFocus {
    /// The configured scope.
    Scope,
    /// Whether a rebuild is running.
    Rebuild,
}

impl SearchFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::Scope => "scope",
            Self::Rebuild => "rebuild",
        }
    }
}

/// A normalized search observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchState {
    /// Which fact this carries.
    pub focus: SearchFocus,
    /// The digest of the configured roots + exclusions, when that is the focus.
    pub scope_digest: Option<String>,
    /// Whether a rebuild is running.
    pub rebuilding: bool,
}

impl NormalizedObservation for SearchState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "search:{}:{}:{}",
            self.focus.tag(),
            self.scope_digest.as_deref().unwrap_or("-"),
            self.rebuilding,
        ))
    }
}

/// Validate one indexed root.
///
/// A root is refused rather than normalised when it is relative or contains `..`:
/// the caller must be able to see exactly what it is widening the index to, and a
/// traversal hides that.
pub fn validate_root(path: &PathBuf) -> Result<(), OsControlError> {
    if !path.is_absolute() {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("roots"),
            reason: SafeText::new("every root must be an absolute path"),
        });
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("roots"),
            reason: SafeText::new(
                "a root must not contain `..`: the widened scope would not match what was approved",
            ),
        });
    }
    Ok(())
}

/// A validated scope change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeChange {
    /// The roots to index.
    pub roots: Vec<PathBuf>,
    /// The paths to exclude.
    pub exclusions: Vec<PathBuf>,
}

impl ScopeChange {
    /// Validate a requested scope change against the frozen bounds.
    pub fn parse(roots: Vec<PathBuf>, exclusions: Vec<PathBuf>) -> Result<Self, OsControlError> {
        if roots.is_empty() || roots.len() > MAX_SCOPE_ROOTS {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("roots"),
                reason: SafeText::new("roots must contain between 1 and 256 paths"),
            });
        }
        if exclusions.len() > MAX_SCOPE_EXCLUSIONS {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("exclusions"),
                reason: SafeText::new("exclusions may contain at most 64 paths"),
            });
        }
        for path in roots.iter().chain(exclusions.iter()) {
            validate_root(path)?;
        }
        Ok(Self { roots, exclusions })
    }

    /// A stable digest of this scope, used as the postcondition.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut roots: Vec<String> = self
            .roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut exclusions: Vec<String> = self
            .exclusions
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        // Sorted so the digest describes the SET, not the order it arrived in.
        roots.sort();
        exclusions.sort();
        Digest::of_str(&format!("{}|{}", roots.join(":"), exclusions.join(":")))
            .as_hex()
            .to_string()
    }
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchOp {
    /// Replace the indexed scope.
    ConfigureScope(ScopeChange),
    /// Rebuild the index for a scope.
    Rebuild {
        /// The scope to rebuild, or the default when absent.
        scope: Option<SearchScopeId>,
    },
}

impl SearchOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> SearchFocus {
        match self {
            Self::ConfigureScope(_) => SearchFocus::Scope,
            Self::Rebuild { .. } => SearchFocus::Rebuild,
        }
    }
}

/// One governed search request.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The operation.
    pub op: SearchOp,
}

impl SearchRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &SearchState) -> SearchState {
        match &self.op {
            SearchOp::ConfigureScope(change) => SearchState {
                focus: SearchFocus::Scope,
                scope_digest: Some(change.digest()),
                rebuilding: observed.rebuilding,
            },
            // A rebuild is verified by the job being ACCEPTED and running, not by
            // the index being complete — completion can take hours, and claiming it
            // finished would be false.
            SearchOp::Rebuild { .. } => SearchState {
                focus: SearchFocus::Rebuild,
                scope_digest: observed.scope_digest.clone(),
                rebuilding: true,
            },
        }
    }

    /// The comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// The raw transport.
#[async_trait]
pub trait SearchTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read the resolved scope.
    async fn read_scope(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<&SearchScopeId>,
    ) -> Result<SearchScope, OsControlError>;

    /// Whether a rebuild is running.
    async fn read_rebuild_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebuildState, OsControlError>;

    /// Run a bounded query.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        scope: &SearchScope,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<SearchPage, OsControlError>;

    /// Apply one operation.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &SearchOp,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct SearchControl<T: SearchTransport> {
    transport: T,
}

impl<T: SearchTransport> SearchControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Clamp a page size to the contract bound.
    #[must_use]
    pub fn page_limit(limit: Option<usize>) -> usize {
        limit.unwrap_or(SEARCH_PAGE_DEFAULT).clamp(1, SEARCH_PAGE_MAX)
    }

    fn satisfying(&self, observed: &SearchState) -> SatisfyingVerification<SearchState> {
        let digest = observed.observation_digest();
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), digest),
            None,
            std::time::SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: SearchTransport> DesiredStateControl<SearchRequest, SearchState> for SearchControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &SearchRequest,
    ) -> Result<SearchState, OsControlError> {
        let rebuilding =
            matches!(self.transport.read_rebuild_state(ctx).await?, RebuildState::Running);
        let scope_digest = match &request.op {
            SearchOp::ConfigureScope(_) => {
                let current = self.transport.read_scope(ctx, None).await?;
                Some(
                    ScopeChange {
                        roots: current.roots,
                        exclusions: current.exclusions,
                    }
                    .digest(),
                )
            }
            SearchOp::Rebuild { .. } => None,
        };
        Ok(SearchState {
            focus: request.op.focus(),
            scope_digest,
            rebuilding,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &SearchRequest,
        _desired: &SearchState,
    ) -> Result<ApplyOutcome, OsControlError> {
        if matches!(request.op, SearchOp::Rebuild { .. }) {
            // Starting a second rebuild while one runs would thrash the disk and
            // neither would finish sooner.
            if matches!(
                self.transport.read_rebuild_state(ctx.observation()).await?,
                RebuildState::Running
            ) {
                return Err(OsControlError::ResourceBusy {
                    resource: crate::os_control::contract::SafeResource::new("search-index"),
                    owner: None,
                });
            }
        }
        self.transport.apply(ctx, &request.op).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &SearchRequest,
        desired: &SearchState,
    ) -> Result<VerificationReport<SearchState>, OsControlError> {
        let observed = self.observe(ctx, request).await?;
        // For a scope change, re-read the NEW scope rather than the pre-state.
        let observed = match &request.op {
            SearchOp::ConfigureScope(_) => {
                let current = self.transport.read_scope(ctx, None).await?;
                SearchState {
                    focus: SearchFocus::Scope,
                    scope_digest: Some(
                        ScopeChange {
                            roots: current.roots,
                            exclusions: current.exclusions,
                        }
                        .digest(),
                    ),
                    rebuilding: observed.rebuilding,
                }
            }
            SearchOp::Rebuild { .. } => observed,
        };

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Neither operation has an inverse: a rebuild cannot be un-run, and the
        // previous scope is not retained by this provider.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("search.rollback"),
            reason: SafeText::new("no search operation has an inverse"),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait SearchControlPort: DesiredStateControl<SearchRequest, SearchState> {
    /// Read the resolved scope.
    async fn scope(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<&SearchScopeId>,
    ) -> Result<SearchScope, OsControlError>;

    /// Run a bounded query against a resolved scope.
    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        scope: Option<&SearchScopeId>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SearchPage, OsControlError>;
}

#[async_trait]
impl<T: SearchTransport> SearchControlPort for SearchControl<T> {
    async fn scope(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<&SearchScopeId>,
    ) -> Result<SearchScope, OsControlError> {
        self.transport.read_scope(ctx, scope).await
    }

    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        scope: Option<&SearchScopeId>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SearchPage, OsControlError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("query"),
                reason: SafeText::new("query is empty or exceeds the length bound"),
            });
        }
        // The scope is RESOLVED before searching, because whether content is
        // indexed decides how sensitive this read is.
        let resolved = self.transport.read_scope(ctx, scope).await?;
        self.transport
            .query(ctx, query, &resolved, cursor, Self::page_limit(limit))
            .await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn a_relative_or_traversing_root_is_refused() {
        assert!(validate_root(&PathBuf::from("relative/path")).is_err());
        assert!(validate_root(&PathBuf::from("/home/user/../etc")).is_err());
        assert!(validate_root(&PathBuf::from("/home/user/Documents")).is_ok());
    }

    #[test]
    fn scope_bounds_are_enforced() {
        assert!(ScopeChange::parse(vec![], vec![]).is_err(), "at least one root");
        let too_many: Vec<PathBuf> = (0..300).map(|i| PathBuf::from(format!("/r{i}"))).collect();
        assert!(ScopeChange::parse(too_many, vec![]).is_err());
        assert!(ScopeChange::parse(vec![PathBuf::from("/home/u")], vec![]).is_ok());
    }

    #[test]
    fn the_scope_digest_describes_the_set_not_the_order() {
        let a = ScopeChange::parse(
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            vec![],
        )
        .unwrap();
        let b = ScopeChange::parse(
            vec![PathBuf::from("/b"), PathBuf::from("/a")],
            vec![],
        )
        .unwrap();
        assert_eq!(a.digest(), b.digest(), "order must not change the identity");
    }

    #[test]
    fn a_rebuild_is_verified_as_running_not_finished() {
        let observed = SearchState {
            focus: SearchFocus::Rebuild,
            scope_digest: None,
            rebuilding: false,
        };
        let request = SearchRequest {
            action: "rebuild_search_index".to_string(),
            params: serde_json::Value::Null,
            op: SearchOp::Rebuild { scope: None },
        };
        let desired = request.desired_state(&observed);
        assert!(
            desired.rebuilding,
            "completion can take hours; only acceptance is observable"
        );
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        let scope = SearchState {
            focus: SearchFocus::Scope,
            scope_digest: Some("abc".to_string()),
            rebuilding: true,
        };
        let rebuild = SearchState {
            focus: SearchFocus::Rebuild,
            ..scope.clone()
        };
        assert_ne!(scope.observation_digest(), rebuild.observation_digest());
    }

    #[test]
    fn an_option_looking_scope_id_is_refused() {
        assert!(SearchScopeId::parse("-x").is_err());
        assert!(SearchScopeId::parse("").is_err());
        assert!(SearchScopeId::parse("home-documents").is_ok());
    }

    #[test]
    fn page_limit_is_clamped() {
        assert_eq!(SearchControl::<Dummy>::page_limit(None), SEARCH_PAGE_DEFAULT);
        assert_eq!(SearchControl::<Dummy>::page_limit(Some(0)), 1);
        assert_eq!(SearchControl::<Dummy>::page_limit(Some(9999)), SEARCH_PAGE_MAX);
    }

    struct Dummy;

    #[async_trait]
    impl SearchTransport for Dummy {
        fn provider_id(&self) -> ProviderId {
            ProviderId::new("dummy")
        }
        async fn read_scope(
            &self,
            _ctx: &HostExecutionContext,
            _scope: Option<&SearchScopeId>,
        ) -> Result<SearchScope, OsControlError> {
            unreachable!("not used")
        }
        async fn read_rebuild_state(
            &self,
            _ctx: &HostExecutionContext,
        ) -> Result<RebuildState, OsControlError> {
            unreachable!("not used")
        }
        async fn query(
            &self,
            _ctx: &HostExecutionContext,
            _query: &str,
            _scope: &SearchScope,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<SearchPage, OsControlError> {
            unreachable!("not used")
        }
        async fn apply(
            &self,
            _ctx: &AdmittedMutationContext<'_>,
            _op: &SearchOp,
        ) -> Result<ApplyOutcome, OsControlError> {
            unreachable!("not used")
        }
    }
}
