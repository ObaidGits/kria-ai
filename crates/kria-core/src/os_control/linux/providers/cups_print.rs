//! The live print provider, backed by CUPS.
//!
//! linux-os-control-production task **5.2**.
//!
//! # Cancelling only your own jobs
//!
//! `lpstat` reports every user's queue. [`PrintJobInfo::owned_by_caller`] is
//! computed by comparing the job's owner against the **effective uid's login
//! name**, resolved from the password database rather than from `$USER` — an
//! environment variable is attacker-controlled, and trusting it would let a
//! manipulated environment mark someone else's job as yours and cancel it.
//!
//! # A print job is verified as QUEUED, not as printed
//!
//! Paper leaving the printer is not observable from software. The domain verifies
//! that CUPS accepted the job, and says so.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::print::{
    PrintJobId, PrintJobInfo, PrintJobPage, PrintOp, PrintTransport, PrinterId, PrinterInfo,
    PrinterPage,
};
use crate::os_control::receipt::ApplyOutcome;

const LPSTAT_PATHS: &[&str] = &["/usr/bin/lpstat"];
const LP_PATHS: &[&str] = &["/usr/bin/lp"];
const CANCEL_PATHS: &[&str] = &["/usr/bin/cancel"];

/// The live CUPS transport.
pub struct LivePrint {
    lpstat: &'static str,
    lp: Option<&'static str>,
    cancel: Option<&'static str>,
    /// The caller's login name, resolved once at composition.
    login: Option<String>,
}

impl LivePrint {
    /// Compose the provider when `lpstat` is present.
    ///
    /// `lpstat` alone is enough to read; submitting and cancelling degrade to
    /// `Unsupported` if their tools are missing, rather than making the whole
    /// domain unavailable and hiding the queue the user asked about.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            lpstat: cli::first_present(LPSTAT_PATHS)?,
            lp: cli::first_present(LP_PATHS),
            cancel: cli::first_present(CANCEL_PATHS),
            login: effective_login_name(),
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("cups")
    }
}

/// The login name of the effective uid, from the password database.
///
/// Deliberately not `$USER`: the environment is caller-controlled, and job
/// ownership decides whether a cancel is allowed.
fn effective_login_name() -> Option<String> {
    // SAFETY: `geteuid` cannot fail. `getpwuid` returns a pointer into a static
    // buffer owned by libc; the name is copied out before any further libc call
    // could overwrite it.
    unsafe {
        let uid = libc::geteuid();
        let entry = libc::getpwuid(uid);
        if entry.is_null() {
            return None;
        }
        let name = (*entry).pw_name;
        if name.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(name)
            .to_str()
            .ok()
            .map(str::to_string)
    }
}

/// Parse `lpstat -p -d` output into printers.
///
/// Lines look like:
/// `printer HP_LaserJet is idle.  enabled since ...`
/// `system default destination: HP_LaserJet`
fn parse_printers(raw: &str) -> Vec<(String, String, bool, bool)> {
    let mut printers: Vec<(String, String, bool, bool)> = Vec::new();
    let mut default_name: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("system default destination:") {
            default_name = Some(rest.trim().to_string());
            continue;
        }
        let Some(rest) = line.strip_prefix("printer ") else {
            continue;
        };
        let mut parts = rest.splitn(2, ' ');
        let Some(name) = parts.next().filter(|name| !name.is_empty()) else {
            continue;
        };
        let remainder = parts.next().unwrap_or("");
        // CUPS words: "is idle", "is printing", "disabled since".
        let state = if remainder.contains("is printing") {
            "printing"
        } else if remainder.contains("is idle") {
            "idle"
        } else if remainder.contains("disabled") {
            "stopped"
        } else {
            // An unrecognized phrasing is reported verbatim rather than mapped to
            // "idle", which would claim a stopped printer is ready.
            "unknown"
        };
        // "enabled since" means it accepts jobs; "rejecting" means it does not.
        let accepting = !remainder.contains("rejecting") && !remainder.contains("disabled");
        printers.push((name.to_string(), state.to_string(), accepting, false));
    }
    if let Some(default_name) = default_name {
        for printer in &mut printers {
            printer.3 = printer.0 == default_name;
        }
    }
    printers
}

/// Parse `lpstat -o` output into jobs.
///
/// Lines look like: `HP_LaserJet-42 alice 1024 Tue 13 Aug 2026 12:00:00`
/// The first token is `<printer>-<job number>`, the second the owner.
fn parse_jobs(raw: &str) -> Vec<(String, String, String, Option<u64>)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else { continue };
        let Some(owner) = parts.next() else { continue };
        // Split at the LAST '-': a printer name may itself contain hyphens.
        let Some(split) = id.rfind('-') else { continue };
        let printer = &id[..split];
        let number = &id[split + 1..];
        if printer.is_empty() || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let size = parts.next().and_then(|token| token.parse::<u64>().ok());
        out.push((id.to_string(), printer.to_string(), owner.to_string(), size));
    }
    out
}

impl LivePrint {
    /// Read the raw job rows once.
    async fn job_rows(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<(String, String, String, Option<u64>)>, OsControlError> {
        // `lpstat -o` exits non-zero on some builds when the queue is empty, which
        // is a legitimate observation.
        let (raw, _exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "print.list_jobs",
            self.lpstat,
            vec!["-o".into()],
        )
        .await?;
        Ok(parse_jobs(&raw))
    }

    /// Build a job info row, deciding ownership against the password database.
    fn job_info(
        &self,
        id: &str,
        printer: &str,
        owner: &str,
        size: Option<u64>,
    ) -> Option<PrintJobInfo> {
        Some(PrintJobInfo {
            job: PrintJobId::parse(id).ok()?,
            printer: PrinterId::parse(printer).ok()?,
            // Unknown login name means ownership cannot be proven, so the job is
            // NOT treated as the caller's. Failing closed here is what stops a
            // cancel from reaching someone else's document.
            owned_by_caller: self.login.as_deref() == Some(owner),
            state: "queued".to_string(),
            size_bytes: size,
        })
    }
}

#[async_trait]
impl PrintTransport for LivePrint {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn list_printers(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<PrinterPage, OsControlError> {
        let (raw, _exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "print.list_printers",
            self.lpstat,
            vec!["-p".into(), "-d".into()],
        )
        .await?;
        let mut all: Vec<PrinterInfo> = Vec::new();
        for (name, state, accepting, is_default) in parse_printers(&raw) {
            // An unparseable name is skipped: it could not be printed to anyway.
            if let Ok(printer) = PrinterId::parse(&name) {
                all.push(PrinterInfo {
                    printer,
                    description: SafeText::new(name),
                    accepting,
                    is_default,
                    state,
                });
            }
        }
        all.sort_by(|a, b| a.printer.cmp(&b.printer));
        let start = match cursor {
            Some(last) => all
                .iter()
                .position(|item| item.printer.as_str() == last)
                .map_or(0, |index| index + 1),
            None => 0,
        };
        let items: Vec<PrinterInfo> = all.iter().skip(start).take(limit).cloned().collect();
        let truncated = start + items.len() < all.len();
        Ok(PrinterPage {
            next_cursor: truncated
                .then(|| items.last().map(|item| item.printer.as_str().to_string()))
                .flatten(),
            items,
            truncated,
        })
    }

    async fn list_jobs(
        &self,
        ctx: &HostExecutionContext,
        printer: Option<&PrinterId>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<PrintJobPage, OsControlError> {
        let rows = self.job_rows(ctx).await?;
        let mut all: Vec<PrintJobInfo> = rows
            .iter()
            .filter_map(|(id, job_printer, owner, size)| {
                self.job_info(id, job_printer, owner, *size)
            })
            .filter(|job| printer.is_none_or(|wanted| job.printer == *wanted))
            .collect();
        all.sort_by(|a, b| a.job.cmp(&b.job));
        let start = match cursor {
            Some(last) => all
                .iter()
                .position(|item| item.job.as_str() == last)
                .map_or(0, |index| index + 1),
            None => 0,
        };
        let items: Vec<PrintJobInfo> = all.iter().skip(start).take(limit).cloned().collect();
        let truncated = start + items.len() < all.len();
        Ok(PrintJobPage {
            next_cursor: truncated
                .then(|| items.last().map(|item| item.job.as_str().to_string()))
                .flatten(),
            items,
            truncated,
        })
    }

    async fn read_job(
        &self,
        ctx: &HostExecutionContext,
        job: &PrintJobId,
    ) -> Result<Option<PrintJobInfo>, OsControlError> {
        let rows = self.job_rows(ctx).await?;
        // `None` means the job is genuinely absent from the queue — which is how
        // the domain verifies a cancel succeeded.
        Ok(rows
            .iter()
            .find(|(id, ..)| id == job.as_str())
            .and_then(|(id, printer, owner, size)| self.job_info(id, printer, owner, *size)))
    }

    async fn read_printer(
        &self,
        ctx: &HostExecutionContext,
        printer: &PrinterId,
    ) -> Result<Option<PrinterInfo>, OsControlError> {
        let page = self.list_printers(ctx, None, 512).await?;
        Ok(page
            .items
            .into_iter()
            .find(|item| item.printer == *printer))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &PrintOp,
    ) -> Result<ApplyOutcome, OsControlError> {
        match op {
            PrintOp::Submit {
                printer,
                path,
                options,
            } => {
                let Some(lp) = self.lp else {
                    return Err(cli::missing(self.id(), "lp (CUPS client)"));
                };
                let path_text = path.to_string_lossy().to_string();
                cli::reject_option_like("path", &path_text)?;
                let mut argv = vec![
                    "-d".into(),
                    printer.as_str().to_string(),
                    "-n".into(),
                    options.copies.to_string(),
                ];
                if options.duplex {
                    argv.push("-o".into());
                    argv.push("sides=two-sided-long-edge".into());
                }
                // `--` terminates options so a filename can never be read as one,
                // in addition to the leading-dash rejection above.
                argv.push("--".into());
                argv.push(path_text);
                cli::dispatch(
                    ctx,
                    "print.submit",
                    lp,
                    argv,
                )
                .await
            }
            PrintOp::CancelOwned { job } => {
                let Some(cancel) = self.cancel else {
                    return Err(cli::missing(self.id(), "cancel (CUPS client)"));
                };
                // Ownership was already proven by the domain against
                // `owned_by_caller` before admission; this only carries it out.
                cli::dispatch(
                    ctx,
                    "print.cancel",
                    cancel,
                    vec![job.as_str().to_string()],
                )
                .await
            }
            PrintOp::Configure { .. } => {
                // `lpadmin` requires administrative rights. Routed through the
                // broker as a typed operation, never by escalating in-process.
                Err(OsControlError::Unsupported {
                    capability: crate::os_control::contract::CapabilityId::new(
                        "print.configure.privileged",
                    ),
                    reason: SafeText::new(
                        "changing a printer's configuration needs administrative rights; install \
                         the KRIA broker service to enable it",
                    ),
                })
            }
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn provider(login: Option<&str>) -> LivePrint {
        LivePrint {
            lpstat: "/usr/bin/lpstat",
            lp: Some("/usr/bin/lp"),
            cancel: Some("/usr/bin/cancel"),
            login: login.map(str::to_string),
        }
    }

    #[test]
    fn printer_states_are_never_rounded_to_idle() {
        let raw = "printer A is idle.  enabled since now\n\
                   printer B disabled since now -\n\
                   printer C is printing.\n\
                   printer D has some new phrasing\n\
                   system default destination: C";
        let rows = parse_printers(raw);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].1, "idle");
        assert_eq!(rows[1].1, "stopped");
        assert!(!rows[1].2, "a disabled printer is not accepting");
        assert_eq!(rows[2].1, "printing");
        // An unknown phrasing must not become "idle".
        assert_eq!(rows[3].1, "unknown");
        assert!(rows[2].3, "C is the default");
        assert!(!rows[0].3);
    }

    #[test]
    fn a_job_id_splits_at_the_last_hyphen() {
        // A printer named with hyphens would break a first-hyphen split.
        let rows = parse_jobs("HP-Laser-Jet-42 alice 2048 Tue\nbad\nX-notanumber bob 1");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "HP-Laser-Jet");
        assert_eq!(rows[0].2, "alice");
        assert_eq!(rows[0].3, Some(2048));
    }

    #[test]
    fn ownership_fails_closed_when_the_login_name_is_unknown() {
        let unknown = provider(None);
        let info = unknown
            .job_info("P-1", "P", "alice", None)
            .expect("row parses");
        // Without a proven login name, no job may be claimed as the caller's —
        // otherwise a cancel could reach another user's document.
        assert!(!info.owned_by_caller);

        let known = provider(Some("alice"));
        assert!(
            known
                .job_info("P-1", "P", "alice", None)
                .expect("row parses")
                .owned_by_caller
        );
        assert!(
            !known
                .job_info("P-2", "P", "bob", None)
                .expect("row parses")
                .owned_by_caller
        );
    }

    #[test]
    fn the_login_name_comes_from_the_password_database() {
        // Must not be satisfied by $USER, which a caller controls.
        let from_db = effective_login_name();
        assert!(
            from_db.is_some(),
            "the effective uid must resolve to a login name"
        );
    }
}
