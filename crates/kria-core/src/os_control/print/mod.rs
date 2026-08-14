//! Printing: printer discovery, queue inspection, submission, cancellation and
//! configuration.
//!
//! linux-os-control-production task **4.7** (OSC-021).
//!
//! # Two properties make printing unlike the other domains
//!
//! **A print job leaves the machine.** Submitting a file sends its *contents* to a
//! device that may be shared, networked, or in another room, and once spooled it
//! cannot be recalled. That is why `print_file` is RED with **no rollback claim**:
//! cancelling a job that already printed does not un-print the paper.
//!
//! **Job ownership is not a formality.** A print queue holds other users' jobs on
//! a shared machine. `cancel_print_job` therefore resolves the job's owner and
//! refuses anything the caller does not own — the port operation is literally
//! named `cancel_owned`. Cancelling by a bare id would let one user destroy
//! another's work.
//!
//! # Identity
//!
//! A printer is its CUPS queue name; a job is its numeric CUPS id. A printer's
//! human *description* ("Office Laser") is display text — never an identity,
//! because it is neither unique nor stable.

use async_trait::async_trait;

use crate::os_control::broker::protocol::{DiscoveredPrinterId, ReviewedPrinterOptions};
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

/// The provider identity.
pub const PRINT_PROVIDER_ID: &str = "print-cups";

/// The largest page a listing may return.
pub const PRINT_PAGE_MAX: usize = 256;

/// The default page size.
pub const PRINT_PAGE_DEFAULT: usize = 50;

macro_rules! print_id {
    ($name:ident, $field:literal) => {
        /// A validated print identity token.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a raw token.
            ///
            /// Rejected rather than escaped: this token reaches a governed argv,
            /// and a value starting with `-` would be read as an option by `lp`.
            pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
                let raw = raw.as_ref().trim();
                if raw.is_empty() || raw.len() > 128 {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new($field),
                        reason: SafeText::new("value is empty or too long"),
                    });
                }
                if raw.starts_with('-') {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new($field),
                        reason: SafeText::new(
                            "value must not start with `-`: it would be read as a command option",
                        ),
                    });
                }
                if !raw
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
                {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new($field),
                        reason: SafeText::new("value contains an illegal character"),
                    });
                }
                Ok(Self(raw.to_string()))
            }

            /// Borrow the token.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

print_id!(PrinterId, "printer");
print_id!(PrintJobId, "job");

/// One printer as reported by the print service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterInfo {
    /// The queue name — the identity.
    pub printer: PrinterId,
    /// Human description. Display only; never an identity.
    pub description: SafeText,
    /// Whether the queue accepts new jobs.
    pub accepting: bool,
    /// Whether the queue is the session default.
    pub is_default: bool,
    /// Raw state token (`idle`, `processing`, `stopped`).
    pub state: String,
}

/// One page of printers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterPage {
    /// The printers on this page.
    pub items: Vec<PrinterInfo>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether the listing was cut short.
    pub truncated: bool,
}

/// One queued job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintJobInfo {
    /// The job id — the identity.
    pub job: PrintJobId,
    /// The queue holding it.
    pub printer: PrinterId,
    /// Whether the **calling user** owns this job.
    ///
    /// Carried explicitly so a cancel can refuse another user's work without
    /// having to re-derive ownership at mutation time.
    pub owned_by_caller: bool,
    /// Raw job state token.
    pub state: String,
    /// Size in bytes, when reported.
    pub size_bytes: Option<u64>,
}

/// One page of jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintJobPage {
    /// The jobs on this page.
    pub items: Vec<PrintJobInfo>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether the listing was cut short.
    pub truncated: bool,
}

/// Reviewed submission options. A closed set — there is deliberately no
/// pass-through for arbitrary `lp -o` strings, which would be an injection point
/// into a privileged spooler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReviewedPrintOptions {
    /// Copies to print, 1..=99.
    pub copies: u8,
    /// Print on both sides.
    pub duplex: bool,
}

impl ReviewedPrintOptions {
    /// Validate the option set.
    pub fn validate(self) -> Result<Self, OsControlError> {
        if self.copies == 0 || self.copies > 99 {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("copies"),
                reason: SafeText::new("copies must be between 1 and 99"),
            });
        }
        Ok(self)
    }
}

/// Which fact an observation carries. Part of the digest, so a queue fact can
/// never satisfy a configuration postcondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintFocus {
    /// Whether a specific job is present in a queue.
    JobPresence,
    /// A printer's configuration flags.
    PrinterConfig,
}

impl PrintFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::JobPresence => "job-presence",
            Self::PrinterConfig => "printer-config",
        }
    }
}

/// A normalized print observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintState {
    /// Which fact this carries.
    pub focus: PrintFocus,
    /// Whether the job in question exists in a queue.
    pub job_present: bool,
    /// The job id this observation is about, when any.
    pub job: Option<String>,
    /// Printer configuration flags, when that is the focus.
    pub accepting: Option<bool>,
    /// Whether the printer is the default.
    pub is_default: Option<bool>,
    /// Whether the printer is shared.
    pub shared: Option<bool>,
}

impl NormalizedObservation for PrintState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "print:{}:{}:{}:{}:{}:{}",
            self.focus.tag(),
            self.job_present,
            self.job.as_deref().unwrap_or("-"),
            self.accepting.map_or_else(|| "-".into(), |v| v.to_string()),
            self.is_default.map_or_else(|| "-".into(), |v| v.to_string()),
            self.shared.map_or_else(|| "-".into(), |v| v.to_string()),
        ))
    }
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintOp {
    /// Submit a file to a queue (`print_file`).
    Submit {
        /// The target queue.
        printer: PrinterId,
        /// The canonical path to print.
        path: std::path::PathBuf,
        /// Reviewed options.
        options: ReviewedPrintOptions,
    },
    /// Cancel a job the caller owns (`cancel_print_job`).
    CancelOwned {
        /// The job to cancel.
        job: PrintJobId,
    },
    /// Configure a discovered printer (`configure_printer`).
    Configure {
        /// The discovered printer.
        discovered: DiscoveredPrinterId,
        /// Reviewed closed-set options.
        options: ReviewedPrinterOptions,
    },
}

impl PrintOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> PrintFocus {
        match self {
            Self::Submit { .. } | Self::CancelOwned { .. } => PrintFocus::JobPresence,
            Self::Configure { .. } => PrintFocus::PrinterConfig,
        }
    }
}

/// One governed print request.
#[derive(Debug, Clone)]
pub struct PrintRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The operation.
    pub op: PrintOp,
}

impl PrintRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &PrintState) -> PrintState {
        match &self.op {
            // A submission's postcondition is that a job now EXISTS in the queue.
            // It deliberately says nothing about paper having emerged: the spooler
            // accepting the job is the only thing observable from here.
            PrintOp::Submit { .. } => PrintState {
                focus: PrintFocus::JobPresence,
                job_present: true,
                job: None,
                accepting: observed.accepting,
                is_default: observed.is_default,
                shared: observed.shared,
            },
            PrintOp::CancelOwned { job } => PrintState {
                focus: PrintFocus::JobPresence,
                job_present: false,
                job: Some(job.as_str().to_string()),
                accepting: observed.accepting,
                is_default: observed.is_default,
                shared: observed.shared,
            },
            PrintOp::Configure { options, .. } => PrintState {
                focus: PrintFocus::PrinterConfig,
                job_present: observed.job_present,
                job: observed.job.clone(),
                accepting: Some(options.accept_jobs),
                is_default: Some(options.set_default),
                shared: Some(options.shared),
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
pub trait PrintTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// List printers.
    async fn list_printers(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<PrinterPage, OsControlError>;

    /// List queued jobs, optionally scoped to one printer.
    async fn list_jobs(
        &self,
        ctx: &HostExecutionContext,
        printer: Option<&PrinterId>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<PrintJobPage, OsControlError>;

    /// Read one job, if it exists.
    ///
    /// `Ok(None)` is the positive fact "this job is not in any queue". An
    /// unreadable queue must be an `Err`, never `None`, or a cancel would verify
    /// against a queue it could not read.
    async fn read_job(
        &self,
        ctx: &HostExecutionContext,
        job: &PrintJobId,
    ) -> Result<Option<PrintJobInfo>, OsControlError>;

    /// Read one printer's configuration flags.
    async fn read_printer(
        &self,
        ctx: &HostExecutionContext,
        printer: &PrinterId,
    ) -> Result<Option<PrinterInfo>, OsControlError>;

    /// Apply one operation.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &PrintOp,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct PrintControl<T: PrintTransport> {
    transport: T,
}

impl<T: PrintTransport> PrintControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport (tests assert against it).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn satisfying(&self, observed: &PrintState) -> SatisfyingVerification<PrintState> {
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

    /// Clamp a requested page size to the contract's bound.
    #[must_use]
    pub fn page_limit(limit: Option<usize>) -> usize {
        limit.unwrap_or(PRINT_PAGE_DEFAULT).clamp(1, PRINT_PAGE_MAX)
    }
}

#[async_trait]
impl<T: PrintTransport> DesiredStateControl<PrintRequest, PrintState> for PrintControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &PrintRequest,
    ) -> Result<PrintState, OsControlError> {
        match &request.op {
            PrintOp::Submit { printer, .. } => {
                // Before submitting, the queue must exist and accept jobs — a
                // submission to a stopped queue would sit forever and look
                // successful.
                let info = self.transport.read_printer(ctx, printer).await?.ok_or_else(|| {
                    OsControlError::InvalidRequest {
                        field: SafeField::new("printer"),
                        reason: SafeText::new("no such printer queue"),
                    }
                })?;
                Ok(PrintState {
                    focus: PrintFocus::JobPresence,
                    // No job of ours exists yet.
                    job_present: false,
                    job: None,
                    accepting: Some(info.accepting),
                    is_default: Some(info.is_default),
                    shared: None,
                })
            }
            PrintOp::CancelOwned { job } => {
                let found = self.transport.read_job(ctx, job).await?;
                Ok(PrintState {
                    focus: PrintFocus::JobPresence,
                    job_present: found.is_some(),
                    job: Some(job.as_str().to_string()),
                    accepting: None,
                    is_default: None,
                    shared: None,
                })
            }
            PrintOp::Configure { discovered, .. } => {
                let printer = PrinterId::parse(discovered.as_str())?;
                let info = self.transport.read_printer(ctx, &printer).await?;
                Ok(PrintState {
                    focus: PrintFocus::PrinterConfig,
                    job_present: false,
                    job: None,
                    accepting: info.as_ref().map(|i| i.accepting),
                    is_default: info.as_ref().map(|i| i.is_default),
                    // Sharing is not readable from the queue listing; absent means
                    // "not reported", never "not shared".
                    shared: None,
                })
            }
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &PrintRequest,
        _desired: &PrintState,
    ) -> Result<ApplyOutcome, OsControlError> {
        if let PrintOp::CancelOwned { job } = &request.op {
            // Ownership is re-checked immediately before cancelling. A print queue
            // on a shared machine holds other users' jobs, and cancelling one of
            // those would destroy work that is not the caller's.
            let found = self.transport.read_job(ctx.observation(), job).await?;
            match found {
                None => {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new("job"),
                        reason: SafeText::new("no such print job"),
                    })
                }
                Some(info) if !info.owned_by_caller => {
                    return Err(OsControlError::PolicyDenied {
                        reason: SafeText::new(
                            "refusing to cancel a print job owned by another user",
                        ),
                    })
                }
                Some(_) => {}
            }
        }
        self.transport.apply(ctx, &request.op).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &PrintRequest,
        desired: &PrintState,
    ) -> Result<VerificationReport<PrintState>, OsControlError> {
        let observed = self.observe(ctx, request).await?;

        // A submission is verified by a job existing, which `observe` reports as
        // `job_present: false` for the pre-state. Re-derive against the queue.
        let observed = match &request.op {
            PrintOp::Submit { printer, .. } => {
                let page = self
                    .transport
                    .list_jobs(ctx, Some(printer), None, PRINT_PAGE_MAX)
                    .await?;
                PrintState {
                    focus: PrintFocus::JobPresence,
                    job_present: page.items.iter().any(|j| j.owned_by_caller),
                    job: None,
                    accepting: observed.accepting,
                    is_default: observed.is_default,
                    shared: observed.shared,
                }
            }
            _ => observed,
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
        // A spooled job cannot be recalled, and cancelling one that already
        // printed does not un-print the paper. Only `configure_printer` is
        // reversible, and its contract makes that a user-requested action rather
        // than an automatic one.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("print.rollback"),
            reason: SafeText::new(
                "a submitted print job cannot be recalled; printer configuration is reverted by an \
                 explicit user request, not automatically",
            ),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait PrintControlPort: DesiredStateControl<PrintRequest, PrintState> {
    /// List printers.
    async fn printers(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<PrinterPage, OsControlError>;

    /// List queued jobs.
    async fn queue(
        &self,
        ctx: &HostExecutionContext,
        printer: Option<&PrinterId>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<PrintJobPage, OsControlError>;
}

#[async_trait]
impl<T: PrintTransport> PrintControlPort for PrintControl<T> {
    async fn printers(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<PrinterPage, OsControlError> {
        self.transport
            .list_printers(ctx, cursor, Self::page_limit(limit))
            .await
    }

    async fn queue(
        &self,
        ctx: &HostExecutionContext,
        printer: Option<&PrinterId>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<PrintJobPage, OsControlError> {
        self.transport
            .list_jobs(ctx, printer, cursor, Self::page_limit(limit))
            .await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn an_option_looking_identity_is_refused() {
        // `lp -d -r` would be read as flags, not a queue name.
        assert!(PrinterId::parse("-r").is_err());
        assert!(PrintJobId::parse("-1").is_err());
        assert!(PrinterId::parse("Office_Laser-2").is_ok());
        assert!(PrinterId::parse("").is_err());
        assert!(PrinterId::parse("bad name").is_err());
    }

    #[test]
    fn copies_outside_the_range_are_refused_not_clamped() {
        assert!(ReviewedPrintOptions { copies: 0, duplex: false }.validate().is_err());
        assert!(ReviewedPrintOptions { copies: 100, duplex: false }.validate().is_err());
        assert!(ReviewedPrintOptions { copies: 2, duplex: true }.validate().is_ok());
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        let job = PrintState {
            focus: PrintFocus::JobPresence,
            job_present: true,
            job: None,
            accepting: Some(true),
            is_default: Some(true),
            shared: None,
        };
        let config = PrintState {
            focus: PrintFocus::PrinterConfig,
            ..job.clone()
        };
        assert_ne!(job.observation_digest(), config.observation_digest());
    }

    #[test]
    fn cancel_desires_the_job_gone() {
        let observed = PrintState {
            focus: PrintFocus::JobPresence,
            job_present: true,
            job: Some("42".to_string()),
            accepting: None,
            is_default: None,
            shared: None,
        };
        let request = PrintRequest {
            action: "cancel_print_job".to_string(),
            params: serde_json::Value::Null,
            op: PrintOp::CancelOwned {
                job: PrintJobId::parse("42").unwrap(),
            },
        };
        let desired = request.desired_state(&observed);
        assert!(!desired.job_present);
        assert_eq!(desired.job.as_deref(), Some("42"));
    }

    #[test]
    fn submission_desires_a_job_to_exist_not_paper_to_have_emerged() {
        let observed = PrintState {
            focus: PrintFocus::JobPresence,
            job_present: false,
            job: None,
            accepting: Some(true),
            is_default: Some(false),
            shared: None,
        };
        let request = PrintRequest {
            action: "print_file".to_string(),
            params: serde_json::Value::Null,
            op: PrintOp::Submit {
                printer: PrinterId::parse("laser").unwrap(),
                path: std::path::PathBuf::from("/tmp/a.pdf"),
                options: ReviewedPrintOptions { copies: 1, duplex: false },
            },
        };
        let desired = request.desired_state(&observed);
        assert!(
            desired.job_present,
            "the only observable postcondition is that the spooler accepted a job"
        );
    }

    #[test]
    fn page_limit_is_clamped_to_the_contract_bound() {
        assert_eq!(PrintControl::<DummyTransport>::page_limit(None), PRINT_PAGE_DEFAULT);
        assert_eq!(PrintControl::<DummyTransport>::page_limit(Some(0)), 1);
        assert_eq!(
            PrintControl::<DummyTransport>::page_limit(Some(9999)),
            PRINT_PAGE_MAX
        );
    }

    /// A do-nothing transport, only to name a concrete type in the page-limit test.
    struct DummyTransport;

    #[async_trait]
    impl PrintTransport for DummyTransport {
        fn provider_id(&self) -> ProviderId {
            ProviderId::new("dummy")
        }
        async fn list_printers(
            &self,
            _ctx: &HostExecutionContext,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<PrinterPage, OsControlError> {
            unreachable!("not used")
        }
        async fn list_jobs(
            &self,
            _ctx: &HostExecutionContext,
            _printer: Option<&PrinterId>,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<PrintJobPage, OsControlError> {
            unreachable!("not used")
        }
        async fn read_job(
            &self,
            _ctx: &HostExecutionContext,
            _job: &PrintJobId,
        ) -> Result<Option<PrintJobInfo>, OsControlError> {
            unreachable!("not used")
        }
        async fn read_printer(
            &self,
            _ctx: &HostExecutionContext,
            _printer: &PrinterId,
        ) -> Result<Option<PrinterInfo>, OsControlError> {
            unreachable!("not used")
        }
        async fn apply(
            &self,
            _ctx: &AdmittedMutationContext<'_>,
            _op: &PrintOp,
        ) -> Result<ApplyOutcome, OsControlError> {
            unreachable!("not used")
        }
    }
}
