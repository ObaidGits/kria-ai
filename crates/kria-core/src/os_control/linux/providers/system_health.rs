//! The live system-health provider: diagnostics, logs and recovery recipes.
//!
//! linux-os-control-production task **4.6**.
//!
//! # Every check answers three ways, never two
//!
//! `Healthy`, `Unhealthy`, or `Undetermined`. The third is the one that matters:
//! if `/proc/meminfo` is unreadable or `systemctl` is absent, this reports
//! `Undetermined` for that subsystem. A false all-clear is worse than no answer,
//! because it stops the user looking for the fault that is actually there.
//!
//! # Why most checks read files instead of running tools
//!
//! Disk, memory and thermal state all live in `/proc` and `/sys`, which any user
//! can read. Reading them directly avoids spawning a process per check and cannot
//! be affected by a tool's output format changing between releases. Only the
//! service check and the log query need a real tool.
//!
//! # Logs are the privileged part
//!
//! The journal contains authentication failures and other users' activity. On a
//! default Ubuntu install a non-root user cannot read the system journal at all,
//! so this reports `Unavailable` with a specific remediation rather than
//! returning an empty page — "no logs" and "not allowed to see logs" are
//! completely different answers.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::health::{
    HealthDomain, HealthFinding, HealthReport, HealthTransport, HealthVerdict, LogLine, LogPage,
    LogQuery, RecoveryRecipe, RecoveryRecipeId,
};
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::receipt::ApplyOutcome;

const SYSTEMCTL_PATHS: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];
const JOURNALCTL_PATHS: &[&str] = &["/usr/bin/journalctl", "/bin/journalctl"];

/// Free-space fraction below which storage is reported unhealthy.
const LOW_DISK_FRACTION: f64 = 0.05;
/// Available-memory fraction below which memory is reported unhealthy.
const LOW_MEMORY_FRACTION: f64 = 0.05;
/// Temperature (milli-degrees C) above which thermal state is unhealthy.
const HOT_MILLI_CELSIUS: i64 = 90_000;

/// The live health transport.
pub struct LiveHealth {
    systemctl: Option<&'static str>,
    journalctl: Option<&'static str>,
}

impl LiveHealth {
    /// Compose the provider.
    ///
    /// Always composes: the file-backed checks work on any Linux system, so the
    /// domain is useful even when neither tool is present. Missing tools surface
    /// as `Undetermined` for the checks that need them, which is the honest
    /// answer, rather than making the whole domain unavailable.
    #[must_use]
    pub fn discover() -> Self {
        Self {
            systemctl: cli::first_present(SYSTEMCTL_PATHS),
            journalctl: cli::first_present(JOURNALCTL_PATHS),
        }
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("system-health")
    }

    /// Disk capacity on the root filesystem.
    fn check_storage(&self) -> HealthFinding {
        match statvfs_free_fraction("/") {
            Some(free) if free < LOW_DISK_FRACTION => HealthFinding {
                domain: HealthDomain::Storage,
                verdict: HealthVerdict::Unhealthy,
                detail: Some(SafeText::new(format!(
                    "root filesystem is {:.0}% full",
                    (1.0 - free) * 100.0
                ))),
            },
            Some(free) => HealthFinding {
                domain: HealthDomain::Storage,
                verdict: HealthVerdict::Healthy,
                detail: Some(SafeText::new(format!("{:.0}% free on /", free * 100.0))),
            },
            None => HealthFinding {
                domain: HealthDomain::Storage,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("could not read filesystem statistics")),
            },
        }
    }

    /// Available memory, using `MemAvailable` rather than `MemFree`.
    ///
    /// `MemFree` excludes reclaimable cache and would report a healthy machine as
    /// nearly out of memory.
    fn check_memory(&self) -> HealthFinding {
        let Some(total) = read_meminfo_kb("MemTotal") else {
            return HealthFinding {
                domain: HealthDomain::Memory,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("could not read /proc/meminfo")),
            };
        };
        let Some(available) = read_meminfo_kb("MemAvailable") else {
            return HealthFinding {
                domain: HealthDomain::Memory,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("kernel did not report MemAvailable")),
            };
        };
        if total == 0 {
            return HealthFinding {
                domain: HealthDomain::Memory,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("kernel reported zero total memory")),
            };
        }
        #[allow(clippy::cast_precision_loss)]
        let fraction = available as f64 / total as f64;
        let verdict = if fraction < LOW_MEMORY_FRACTION {
            HealthVerdict::Unhealthy
        } else {
            HealthVerdict::Healthy
        };
        HealthFinding {
            domain: HealthDomain::Memory,
            verdict,
            detail: Some(SafeText::new(format!(
                "{:.0}% of memory available",
                fraction * 100.0
            ))),
        }
    }

    /// The hottest thermal zone.
    fn check_thermal(&self) -> HealthFinding {
        let mut hottest: Option<i64> = None;
        if let Ok(entries) = std::fs::read_dir("/sys/class/thermal") {
            for entry in entries.flatten() {
                let path = entry.path().join("temp");
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(value) = text.trim().parse::<i64>() {
                        hottest = Some(hottest.map_or(value, |current: i64| current.max(value)));
                    }
                }
            }
        }
        match hottest {
            Some(milli) if milli > HOT_MILLI_CELSIUS => HealthFinding {
                domain: HealthDomain::Thermal,
                verdict: HealthVerdict::Unhealthy,
                detail: Some(SafeText::new(format!("hottest sensor {}°C", milli / 1000))),
            },
            Some(milli) => HealthFinding {
                domain: HealthDomain::Thermal,
                verdict: HealthVerdict::Healthy,
                detail: Some(SafeText::new(format!("hottest sensor {}°C", milli / 1000))),
            },
            // No thermal zones is not a fault — many desktops expose none.
            None => HealthFinding {
                domain: HealthDomain::Thermal,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("no thermal sensors are exposed")),
            },
        }
    }

    /// Default-route presence. Deliberately does NOT send traffic.
    ///
    /// A reachability probe would contact a third party from the user's machine
    /// without them asking. Having a default route is the strongest claim that can
    /// be made from local state alone, and the detail text says so.
    fn check_network(&self) -> HealthFinding {
        match std::fs::read_to_string("/proc/net/route") {
            Ok(text) => {
                // Column 1 is the destination; `00000000` is the default route.
                let has_default = text
                    .lines()
                    .skip(1)
                    .filter_map(|line| line.split_whitespace().nth(1))
                    .any(|dest| dest == "00000000");
                HealthFinding {
                    domain: HealthDomain::Network,
                    verdict: if has_default {
                        HealthVerdict::Healthy
                    } else {
                        HealthVerdict::Unhealthy
                    },
                    detail: Some(SafeText::new(if has_default {
                        "a default route is configured (not an internet reachability test)"
                    } else {
                        "no default route is configured"
                    })),
                }
            }
            Err(_) => HealthFinding {
                domain: HealthDomain::Network,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("could not read the kernel routing table")),
            },
        }
    }

    /// Failed systemd units.
    async fn check_services(&self, ctx: &HostExecutionContext) -> HealthFinding {
        let Some(systemctl) = self.systemctl else {
            return HealthFinding {
                domain: HealthDomain::Services,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("systemctl is not available")),
            };
        };
        let outcome = cli::query_tolerant(
            ctx,
            self.id(),
            "health.check_services",
            systemctl,
            vec![
                "list-units".into(),
                "--failed".into(),
                "--no-legend".into(),
                "--no-pager".into(),
                "--plain".into(),
            ],
        )
        .await;
        match outcome {
            Ok((raw, true)) => {
                let failed: Vec<&str> = raw
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect();
                if failed.is_empty() {
                    HealthFinding {
                        domain: HealthDomain::Services,
                        verdict: HealthVerdict::Healthy,
                        detail: Some(SafeText::new("no failed units")),
                    }
                } else {
                    HealthFinding {
                        domain: HealthDomain::Services,
                        verdict: HealthVerdict::Unhealthy,
                        detail: Some(SafeText::new(format!("{} failed unit(s)", failed.len()))),
                    }
                }
            }
            // The tool ran but failed, or could not run: unknown, not healthy.
            Ok((_, false)) | Err(_) => HealthFinding {
                domain: HealthDomain::Services,
                verdict: HealthVerdict::Undetermined,
                detail: Some(SafeText::new("could not enumerate failed units")),
            },
        }
    }
}

/// Free-space fraction of the filesystem containing `path`.
fn statvfs_free_fraction(path: &str) -> Option<f64> {
    let c_path = std::ffi::CString::new(path).ok()?;
    // SAFETY: `stat` is a valid zeroed statvfs and `c_path` is a valid NUL-
    // terminated string that outlives the call.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &raw mut stat) != 0 {
            return None;
        }
        stat
    };
    if stat.f_blocks == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    // `f_bavail` is space available to an unprivileged user, which is what the
    // user will actually be able to use — `f_bfree` includes root's reserve.
    Some(stat.f_bavail as f64 / stat.f_blocks as f64)
}

/// Read one `/proc/meminfo` field in kB.
fn read_meminfo_kb(field: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == field {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Parse one journald JSON record into a log line.
///
/// Returns `None` for a record missing a timestamp or message: a line with a
/// fabricated timestamp would sort into the wrong place and mislead a reader
/// about when an event happened.
fn parse_journal_record(line: &str) -> Option<LogLine> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    // journald renders the timestamp as a string of microseconds.
    let timestamp_us: u64 = value
        .get("__REALTIME_TIMESTAMP")?
        .as_str()
        .and_then(|raw| raw.parse().ok())?;
    let message = match value.get("MESSAGE")? {
        serde_json::Value::String(text) => text.clone(),
        // A binary message is rendered as a byte array. Report its presence
        // rather than guessing an encoding.
        serde_json::Value::Array(bytes) => {
            format!("<{} bytes of binary log data>", bytes.len())
        }
        _ => return None,
    };
    let priority = value
        .get("PRIORITY")
        .and_then(|value| match value {
            serde_json::Value::String(text) => text.parse::<u8>().ok(),
            serde_json::Value::Number(number) => number.as_u64().and_then(|v| u8::try_from(v).ok()),
            _ => None,
        })
        // Absent priority means the sender did not set one. 6 (informational) is
        // journald's own documented default for such records.
        .unwrap_or(6);
    Some(LogLine {
        timestamp: timestamp_us / 1_000_000,
        unit: value
            .get("_SYSTEMD_UNIT")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        priority,
        message: SafeText::new(message),
    })
}

#[async_trait]
impl HealthTransport for LiveHealth {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn diagnose(
        &self,
        ctx: &HostExecutionContext,
        scope: Option<HealthDomain>,
    ) -> Result<HealthReport, OsControlError> {
        let wanted = |domain: HealthDomain| scope.is_none_or(|only| only == domain);
        let mut findings = Vec::new();
        if wanted(HealthDomain::Storage) {
            findings.push(self.check_storage());
        }
        if wanted(HealthDomain::Memory) {
            findings.push(self.check_memory());
        }
        if wanted(HealthDomain::Services) {
            findings.push(self.check_services(ctx).await);
        }
        if wanted(HealthDomain::Thermal) {
            findings.push(self.check_thermal());
        }
        if wanted(HealthDomain::Network) {
            findings.push(self.check_network());
        }
        Ok(HealthReport { findings })
    }

    async fn query_logs(
        &self,
        ctx: &HostExecutionContext,
        query: &LogQuery,
    ) -> Result<LogPage, OsControlError> {
        let Some(journalctl) = self.journalctl else {
            return Err(cli::missing(self.id(), "journalctl"));
        };
        let mut argv = vec![
            "--no-pager".into(),
            "--output=json".into(),
            format!("--since=-{}h", query.since_hours),
            format!("--lines={}", query.max_lines),
            format!("--priority={}", query.max_priority),
        ];
        if let Some(unit) = &query.unit {
            cli::reject_option_like("unit", unit)?;
            argv.push(format!("--unit={unit}"));
        }
        let (raw, exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "health.query_logs",
            journalctl,
            argv,
        )
        .await?;
        if !exit_ok {
            // Almost always a permission failure: the system journal is readable
            // only by root and the `systemd-journal` group. Say so, rather than
            // returning an empty page that reads as "nothing happened".
            return Err(OsControlError::PermissionDenied {
                authority: SafeText::new("systemd-journald"),
                remediation: SafeText::new(
                    "reading the system journal requires membership of the 'systemd-journal' group",
                ),
            });
        }
        let mut lines = Vec::new();
        let mut skipped = false;
        for record in raw.lines().filter(|line| !line.trim().is_empty()) {
            match parse_journal_record(record) {
                Some(line) => lines.push(line),
                None => skipped = true,
            }
        }
        Ok(LogPage {
            lines,
            // A record we could not parse is reported as truncation rather than
            // silently dropped: the caller is told the page is incomplete.
            truncated: skipped,
        })
    }

    async fn read_recipe_applied(
        &self,
        _ctx: &HostExecutionContext,
        recipe: &RecoveryRecipeId,
    ) -> Result<bool, OsControlError> {
        // No in-tree recipe exists yet, so no recipe can be reported as applied.
        // Returning `false` for an unknown id would let a caller believe the id
        // was recognized and merely not yet run.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("health.recovery_recipe"),
            reason: SafeText::new(format!(
                "no in-tree recovery recipe is registered under '{}'",
                recipe.as_str()
            )),
        })
    }

    async fn run_recipe(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        recipe: &RecoveryRecipe,
    ) -> Result<ApplyOutcome, OsControlError> {
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("health.recovery_recipe"),
            reason: SafeText::new(format!(
                "recipe '{}' has no in-tree implementation",
                recipe.recipe.as_str()
            )),
        })
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn meminfo_reads_the_named_field_only() {
        // MemAvailable must not be satisfied by MemFree: the two differ by the
        // reclaimable page cache, often by gigabytes.
        assert!(read_meminfo_kb("MemTotal").is_some_and(|kb| kb > 0));
        assert!(read_meminfo_kb("NoSuchFieldXyz").is_none());
    }

    #[test]
    fn root_filesystem_fraction_is_a_fraction() {
        let free = statvfs_free_fraction("/").expect("root filesystem must be readable");
        assert!((0.0..=1.0).contains(&free), "got {free}");
        assert!(statvfs_free_fraction("/definitely/not/a/path").is_none());
    }

    #[test]
    fn a_journal_record_without_a_timestamp_is_skipped() {
        assert!(parse_journal_record(r#"{"MESSAGE":"hi"}"#).is_none());
        assert!(parse_journal_record("not json").is_none());
        let line = parse_journal_record(
            r#"{"__REALTIME_TIMESTAMP":"1700000000000000","MESSAGE":"hi","PRIORITY":"3"}"#,
        )
        .expect("a complete record parses");
        assert_eq!(line.timestamp, 1_700_000_000);
        assert_eq!(line.priority, 3);
    }

    #[test]
    fn a_binary_message_reports_its_size_rather_than_guessing_text() {
        let line = parse_journal_record(
            r#"{"__REALTIME_TIMESTAMP":"1700000000000000","MESSAGE":[104,105]}"#,
        )
        .expect("a binary record still parses");
        assert!(line.message.as_str().contains("2 bytes"));
    }

    #[tokio::test]
    async fn every_file_backed_check_returns_a_verdict() {
        let health = LiveHealth::discover();
        // These read /proc and /sys directly, so they work in any environment and
        // must never panic — an unreadable source becomes Undetermined.
        for finding in [
            health.check_storage(),
            health.check_memory(),
            health.check_thermal(),
            health.check_network(),
        ] {
            assert!(finding.detail.is_some(), "{:?} lacks detail", finding.domain);
        }
    }
}
