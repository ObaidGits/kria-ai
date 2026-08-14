//! Typed automation values (Task 4.5, OSC-027/OSC-028).
//!
//! Everything a scheduled task can *be* is a closed type here. There is
//! deliberately **no** way to express "run this command later":
//!
//! * A schedule is a [`TypedSchedule`] — one of three closed shapes with
//!   bounded fields. It is never a cron expression the caller wrote.
//! * An action is a [`CanonicalAction`] — a canonical `os.<tool>` operation id
//!   that must exist in the **frozen manifest**, plus canonical parameters bound
//!   by digest. A shell string can never be an operation id, so a persisted task
//!   cannot outlive the session as an arbitrary-execution hole.
//!
//! The previous `create_scheduled_task`/`delete_scheduled_task` handlers spawned
//! `crontab` directly with a caller-supplied command line and no policy, grant,
//! lease, audit or verification. They were deleted for that reason. This module
//! is the typed replacement: the *only* representable action is an operation the
//! frozen manifest already governs.

use std::collections::BTreeSet;

use base64::Engine as _;

use crate::os_control::contract::{Digest, SafeField, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::manifest::{frozen_contract, TargetPolicy};
use crate::safety::RiskLevel;

/// Maximum characters in a bounded automation identifier (frozen
/// `AutomationId`/`WorkflowId`: `maxLength` 128).
pub const AUTOMATION_ID_MAX_CHARS: usize = 128;

/// Maximum decoded size of canonical CBOR parameters (frozen
/// `CanonicalCapabilityInvocation.canonical_parameters_cbor`
/// `x-decodedMaxBytes`).
pub const CANONICAL_PARAMS_MAX_DECODED_BYTES: usize = 1_048_576;

/// Maximum encoded (base64) length of canonical CBOR parameters (frozen
/// `maxLength`).
pub const CANONICAL_PARAMS_MAX_ENCODED_CHARS: usize = 1_398_104;

/// Minimum interval between runs, in seconds (frozen `TypedSchedule.interval`
/// `every_seconds.minimum`).
pub const SCHEDULE_INTERVAL_MIN_SECONDS: u64 = 60;

/// Maximum interval between runs, in seconds (frozen `maximum`).
pub const SCHEDULE_INTERVAL_MAX_SECONDS: u64 = 31_536_000;

/// Largest millisecond timestamp the frozen schema accepts.
pub const MAX_EPOCH_MS: u64 = 9_007_199_254_740_991;

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

/// Strip control characters and bound a caller-supplied identifier.
fn sanitize_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(AUTOMATION_ID_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= AUTOMATION_ID_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

macro_rules! automation_id {
    ($name:ident, $field:literal) => {
        impl $name {
            /// Construct a bounded, control-char-free identifier.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_id(&raw.into()))
            }

            /// Construct and reject an empty identifier — an unnamed target is
            /// not an identity, so it must never reach a provider.
            pub fn parse(raw: impl Into<String>) -> Result<Self, OsControlError> {
                let id = Self::new(raw);
                if id.0.is_empty() {
                    return Err(invalid(
                        $field,
                        concat!($field, " must be a non-empty stable identifier"),
                    ));
                }
                Ok(id)
            }

            /// Borrow the identifier.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// A correlation-safe digest of the identifier.
            #[must_use]
            pub fn digest(&self) -> Digest {
                Digest::of_str(&self.0)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
    };
}

/// A scheduled task's stable identity (frozen `AutomationId`).
///
/// A task's *display name* is deliberately not an identity: it is neither
/// unique nor stable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AutomationId(String);

/// A workflow's stable identity (frozen `WorkflowId`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkflowId(String);

automation_id!(AutomationId, "task_id");
automation_id!(WorkflowId, "workflow_id");

/// A provider-assigned configuration revision (frozen `Revision`).
///
/// Compared for **equality** before a mutation: a task whose configuration
/// changed since the caller read it must not be patched against a stale view.
pub type Revision = u64;

// ─────────────────────────────────────────────────────────────────────────────
// Typed schedule
// ─────────────────────────────────────────────────────────────────────────────

/// A day of the week (frozen `TypedSchedule.weekly.weekdays`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    /// The stable snake_case token from the frozen enum.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Weekday::Monday => "monday",
            Weekday::Tuesday => "tuesday",
            Weekday::Wednesday => "wednesday",
            Weekday::Thursday => "thursday",
            Weekday::Friday => "friday",
            Weekday::Saturday => "saturday",
            Weekday::Sunday => "sunday",
        }
    }

    /// Parse the frozen token. An unrecognised weekday is an error, never a
    /// silent default.
    pub fn parse(raw: &str) -> Result<Self, OsControlError> {
        match raw {
            "monday" => Ok(Weekday::Monday),
            "tuesday" => Ok(Weekday::Tuesday),
            "wednesday" => Ok(Weekday::Wednesday),
            "thursday" => Ok(Weekday::Thursday),
            "friday" => Ok(Weekday::Friday),
            "saturday" => Ok(Weekday::Saturday),
            "sunday" => Ok(Weekday::Sunday),
            _ => Err(invalid(
                "patch.schedule.weekdays",
                "weekday must be one of monday..sunday",
            )),
        }
    }

    /// The systemd `OnCalendar` day abbreviation.
    #[must_use]
    pub const fn systemd_abbrev(self) -> &'static str {
        match self {
            Weekday::Monday => "Mon",
            Weekday::Tuesday => "Tue",
            Weekday::Wednesday => "Wed",
            Weekday::Thursday => "Thu",
            Weekday::Friday => "Fri",
            Weekday::Saturday => "Sat",
            Weekday::Sunday => "Sun",
        }
    }
}

/// A closed, bounded schedule (frozen `TypedSchedule`).
///
/// There is no free-form variant: a caller cannot supply a cron expression, a
/// systemd calendar string, or anything else that is interpreted downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedSchedule {
    /// Run exactly once at a wall-clock instant.
    Once {
        /// Unix epoch milliseconds.
        run_at_ms: u64,
    },
    /// Run repeatedly on a bounded interval.
    Interval {
        /// Seconds between runs (60..=31_536_000).
        every_seconds: u64,
        /// Optional first-run instant, Unix epoch milliseconds.
        start_at_ms: Option<u64>,
    },
    /// Run on given weekdays at a local time in an explicit zone.
    Weekly {
        /// Non-empty, de-duplicated weekday set.
        weekdays: BTreeSet<Weekday>,
        /// Hour of day, 0..=23.
        hour: u8,
        /// Minute of hour, 0..=59.
        minute: u8,
        /// IANA zone name (explicit: an implied local zone is not reproducible).
        timezone: String,
    },
}

/// Validate an IANA-style zone name without a regex engine.
///
/// Mirrors the frozen pattern `^[A-Za-z_+-]+(?:/[A-Za-z0-9_+-]+)+$`: at least
/// one `/`, no empty segment, and a restricted character set. Rejected rather
/// than normalised, because a zone we cannot name exactly cannot be reproduced.
fn validate_timezone(raw: &str) -> Result<(), OsControlError> {
    let field = "patch.schedule.timezone";
    if raw.is_empty() || raw.len() > 64 {
        return Err(invalid(field, "timezone must be 1..=64 bytes"));
    }
    let mut segments = raw.split('/');
    let Some(first) = segments.next() else {
        return Err(invalid(field, "timezone must be an IANA Area/Location name"));
    };
    if first.is_empty()
        || !first
            .chars()
            .all(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '+' | '-'))
    {
        return Err(invalid(field, "timezone area must be alphabetic"));
    }
    let mut had_location = false;
    for segment in segments {
        had_location = true;
        if segment.is_empty()
            || !segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
        {
            return Err(invalid(field, "timezone location segment is not permitted"));
        }
    }
    if !had_location {
        return Err(invalid(field, "timezone must be an IANA Area/Location name"));
    }
    Ok(())
}

fn epoch_ms(value: &serde_json::Value, field: &str) -> Result<u64, OsControlError> {
    let ms = value
        .as_u64()
        .ok_or_else(|| invalid(field, "must be a non-negative integer millisecond timestamp"))?;
    if ms > MAX_EPOCH_MS {
        return Err(invalid(field, "millisecond timestamp exceeds the maximum"));
    }
    Ok(ms)
}

fn reject_unknown_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    field: &str,
) -> Result<(), OsControlError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(field, "object contains a property the closed schema does not define"));
        }
    }
    Ok(())
}

impl TypedSchedule {
    /// Parse and validate the frozen `TypedSchedule` shape.
    ///
    /// Every bound in the frozen schema is enforced here; an out-of-range field
    /// is rejected rather than clamped, because a clamped schedule is not the
    /// schedule the caller was approved for.
    pub fn parse(value: &serde_json::Value) -> Result<Self, OsControlError> {
        let field = "patch.schedule";
        let object = value
            .as_object()
            .ok_or_else(|| invalid(field, "schedule must be an object"))?;
        let kind = object
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid(field, "schedule requires a `kind` of once|interval|weekly"))?;

        match kind {
            "once" => {
                reject_unknown_keys(object, &["kind", "run_at_ms"], field)?;
                let run_at_ms = epoch_ms(
                    object
                        .get("run_at_ms")
                        .ok_or_else(|| invalid(field, "once schedule requires `run_at_ms`"))?,
                    "patch.schedule.run_at_ms",
                )?;
                Ok(TypedSchedule::Once { run_at_ms })
            }
            "interval" => {
                reject_unknown_keys(object, &["kind", "every_seconds", "start_at_ms"], field)?;
                let every_seconds = object
                    .get("every_seconds")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| {
                        invalid(
                            "patch.schedule.every_seconds",
                            "interval schedule requires an integer `every_seconds`",
                        )
                    })?;
                if !(SCHEDULE_INTERVAL_MIN_SECONDS..=SCHEDULE_INTERVAL_MAX_SECONDS)
                    .contains(&every_seconds)
                {
                    return Err(invalid(
                        "patch.schedule.every_seconds",
                        "every_seconds must be between 60 and 31536000",
                    ));
                }
                let start_at_ms = match object.get("start_at_ms") {
                    None => None,
                    Some(raw) => Some(epoch_ms(raw, "patch.schedule.start_at_ms")?),
                };
                Ok(TypedSchedule::Interval {
                    every_seconds,
                    start_at_ms,
                })
            }
            "weekly" => {
                reject_unknown_keys(
                    object,
                    &["kind", "weekdays", "hour", "minute", "timezone"],
                    field,
                )?;
                let raw_days = object
                    .get("weekdays")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| {
                        invalid(
                            "patch.schedule.weekdays",
                            "weekly schedule requires a `weekdays` array",
                        )
                    })?;
                if raw_days.is_empty() || raw_days.len() > 7 {
                    return Err(invalid(
                        "patch.schedule.weekdays",
                        "weekdays must contain between 1 and 7 entries",
                    ));
                }
                let mut weekdays = BTreeSet::new();
                for day in raw_days {
                    let token = day.as_str().ok_or_else(|| {
                        invalid("patch.schedule.weekdays", "weekday must be a string token")
                    })?;
                    // `uniqueItems: true` — a duplicate is a malformed request,
                    // not something to silently collapse.
                    if !weekdays.insert(Weekday::parse(token)?) {
                        return Err(invalid(
                            "patch.schedule.weekdays",
                            "weekdays must not repeat a day",
                        ));
                    }
                }
                let hour = object
                    .get("hour")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|h| *h <= 23)
                    .ok_or_else(|| invalid("patch.schedule.hour", "hour must be 0..=23"))?
                    as u8;
                let minute = object
                    .get("minute")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|m| *m <= 59)
                    .ok_or_else(|| invalid("patch.schedule.minute", "minute must be 0..=59"))?
                    as u8;
                let timezone = object
                    .get("timezone")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        invalid(
                            "patch.schedule.timezone",
                            "weekly schedule requires an explicit IANA `timezone`",
                        )
                    })?
                    .to_string();
                validate_timezone(&timezone)?;
                Ok(TypedSchedule::Weekly {
                    weekdays,
                    hour,
                    minute,
                    timezone,
                })
            }
            _ => Err(invalid(
                field,
                "schedule `kind` must be once, interval or weekly",
            )),
        }
    }

    /// The canonical, digest-stable rendering of this schedule.
    ///
    /// Used as the postcondition fact for a schedule patch: two schedules are
    /// the same state exactly when this rendering matches.
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            TypedSchedule::Once { run_at_ms } => format!("once:{run_at_ms}"),
            TypedSchedule::Interval {
                every_seconds,
                start_at_ms,
            } => format!(
                "interval:{every_seconds}:{}",
                start_at_ms
                    .map(|ms| ms.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ),
            TypedSchedule::Weekly {
                weekdays,
                hour,
                minute,
                timezone,
            } => {
                let days = weekdays
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("weekly:{days}:{hour:02}:{minute:02}:{timezone}")
            }
        }
    }

    /// The digest of the canonical rendering.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&self.canonical())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Canonical action
// ─────────────────────────────────────────────────────────────────────────────

/// A validated `CanonicalCapabilityInvocation` — the **only** action a
/// scheduled task can carry.
///
/// The action is closed by construction: `operation_id` must name an operation
/// that already exists in the frozen manifest, so there is no representable
/// "run this command" action. A BLACK operation is refused outright (the frozen
/// risk rules make it a `ValidationFailure`, not a stricter approval).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAction {
    operation_id: String,
    tool_name: String,
    input_schema_digest: Digest,
    parameter_digest: Digest,
    parameters_len: usize,
    risk: RiskLevel,
}

/// Operation-name fragments that would reintroduce arbitrary execution if one
/// ever entered the frozen manifest. Defence in depth: the manifest contains no
/// such operation today, and this list means adding one cannot silently make
/// persisted automation an execution hole.
const EXECUTION_SHAPED_NAMES: [&str; 6] = [
    "shell",
    "exec",
    "eval",
    "run_command",
    "run_script",
    "spawn",
];

impl CanonicalAction {
    /// Validate the frozen `CanonicalCapabilityInvocation` shape.
    pub fn parse(value: &serde_json::Value) -> Result<Self, OsControlError> {
        let field = "patch.action";
        let object = value
            .as_object()
            .ok_or_else(|| invalid(field, "action must be an object"))?;
        reject_unknown_keys(
            object,
            &[
                "operation_id",
                "target",
                "input_schema_digest",
                "canonical_parameters_cbor",
                "parameter_digest",
            ],
            field,
        )?;

        let operation_id = object
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid(field, "action requires an `operation_id`"))?;
        validate_operation_id_shape(operation_id)?;

        // `target` is a const in the frozen schema; a different target is not a
        // stricter request, it is an invalid one.
        match object.get("target").and_then(serde_json::Value::as_str) {
            Some("HostLocalOnly") => {}
            _ => {
                return Err(invalid(
                    "patch.action.target",
                    "action target must be HostLocalOnly",
                ))
            }
        }

        let tool_name = operation_id
            .strip_prefix("os.")
            .expect("shape validated above")
            .to_string();

        // The closed-set check. An operation absent from the frozen manifest can
        // never be scheduled, so a persisted task cannot outlive the session as
        // an ungoverned capability.
        let contract = frozen_contract(&tool_name).ok_or_else(|| {
            invalid(
                "patch.action.operation_id",
                "operation_id is not a canonical operation in the frozen manifest",
            )
        })?;
        if contract.target != TargetPolicy::HostLocalOnly {
            return Err(invalid(
                "patch.action.operation_id",
                "operation is not host-local and cannot be scheduled",
            ));
        }
        if EXECUTION_SHAPED_NAMES
            .iter()
            .any(|needle| tool_name.contains(needle))
        {
            return Err(invalid(
                "patch.action.operation_id",
                "an operation that executes caller-supplied code may never be scheduled",
            ));
        }
        let risk = contract.default_tier();
        if risk == RiskLevel::Black {
            // Frozen risk rule: BLACK resolves to ValidationFailure, never to a
            // stricter approval path.
            return Err(invalid(
                "patch.action.operation_id",
                "a BLACK operation may never be scheduled",
            ));
        }

        let encoded = object
            .get("canonical_parameters_cbor")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                invalid(field, "action requires base64 `canonical_parameters_cbor`")
            })?;
        if encoded.is_empty() || encoded.len() > CANONICAL_PARAMS_MAX_ENCODED_CHARS {
            return Err(invalid(
                "patch.action.canonical_parameters_cbor",
                "encoded parameters are empty or exceed the maximum length",
            ));
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| {
                invalid(
                    "patch.action.canonical_parameters_cbor",
                    "canonical parameters are not valid base64",
                )
            })?;
        if decoded.is_empty() || decoded.len() > CANONICAL_PARAMS_MAX_DECODED_BYTES {
            return Err(invalid(
                "patch.action.canonical_parameters_cbor",
                "decoded parameters are empty or exceed the decoded byte maximum",
            ));
        }

        // Both digests are *bindings*, not decoration: the parameters digest
        // must match the bytes supplied, and the schema digest must match the
        // schema this build froze for that operation. A caller cannot bind an
        // action to a schema version this build does not have.
        let parameter_digest = expect_digest(object, "parameter_digest")?;
        if parameter_digest != Digest::of_bytes(&decoded) {
            return Err(invalid(
                "patch.action.parameter_digest",
                "parameter_digest does not match the supplied canonical parameters",
            ));
        }
        let input_schema_digest = expect_digest(object, "input_schema_digest")?;
        let expected_schema_digest = input_schema_digest_for(contract);
        if input_schema_digest != expected_schema_digest {
            return Err(invalid(
                "patch.action.input_schema_digest",
                "input_schema_digest does not match this build's frozen input schema for the operation",
            ));
        }

        Ok(Self {
            operation_id: operation_id.to_string(),
            tool_name,
            input_schema_digest,
            parameter_digest,
            parameters_len: decoded.len(),
            risk,
        })
    }

    /// The canonical operation id (`os.<tool>`).
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// The canonical tool name.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// The contained action's risk, as resolved from the frozen manifest. This
    /// is the `resulting_contained_action_risk` the frozen risk rules for
    /// `modify_scheduled_task` are written against.
    #[must_use]
    pub fn risk(&self) -> RiskLevel {
        self.risk
    }

    /// Decoded parameter length in bytes. The parameter *bytes* are never
    /// retained here: only their length and digest, so a scheduled action can
    /// never leak a payload through a log line or a receipt.
    #[must_use]
    pub fn parameters_len(&self) -> usize {
        self.parameters_len
    }

    /// The stable action identity used as the postcondition fact.
    #[must_use]
    pub fn action_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "{}:{}:{}",
            self.operation_id,
            self.input_schema_digest.as_hex(),
            self.parameter_digest.as_hex()
        ))
    }
}

/// The digest of the frozen input schema for a canonical operation.
///
/// Computed from the embedded manifest so callers and the provider agree
/// without a second source of truth.
#[must_use]
pub fn input_schema_digest_for(
    contract: &crate::os_control::manifest::ToolContractMetadata,
) -> Digest {
    Digest::of_str(
        &serde_json::to_string(&contract.input_schema)
            .unwrap_or_else(|_| "unserializable".to_string()),
    )
}

fn expect_digest(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Digest, OsControlError> {
    let raw = object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid(key, "a 64-character lower-case hex digest is required"))?;
    if raw.len() != 64 || !raw.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()) {
        return Err(invalid(key, "digest must be 64 lower-case hex characters"));
    }
    Ok(Digest::from_hex(raw))
}

/// Enforce the frozen `operation_id` pattern `^os\.[a-z][a-z0-9_]*$`.
fn validate_operation_id_shape(raw: &str) -> Result<(), OsControlError> {
    let field = "patch.action.operation_id";
    if raw.len() < 4 || raw.len() > 128 {
        return Err(invalid(field, "operation_id must be 4..=128 bytes"));
    }
    let Some(rest) = raw.strip_prefix("os.") else {
        return Err(invalid(field, "operation_id must start with `os.`"));
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => {
            return Err(invalid(
                field,
                "operation_id must continue with a lower-case letter",
            ))
        }
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(invalid(
            field,
            "operation_id may contain only lower-case letters, digits and underscores",
        ));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed patch
// ─────────────────────────────────────────────────────────────────────────────

/// A validated `TypedAutomationPatch`: at least one of schedule, action or
/// enabled, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypedAutomationPatch {
    /// The replacement schedule, when the patch changes it.
    pub schedule: Option<TypedSchedule>,
    /// The replacement action, when the patch changes it.
    pub action: Option<CanonicalAction>,
    /// The replacement enabled flag, when the patch changes it.
    pub enabled: Option<bool>,
}

impl TypedAutomationPatch {
    /// Parse and validate the frozen `TypedAutomationPatch` shape
    /// (`additionalProperties: false`, `minProperties: 1`).
    pub fn parse(value: &serde_json::Value) -> Result<Self, OsControlError> {
        let field = "patch";
        let object = value
            .as_object()
            .ok_or_else(|| invalid(field, "patch must be an object"))?;
        reject_unknown_keys(object, &["schedule", "action", "enabled"], field)?;
        if object.is_empty() {
            return Err(invalid(field, "patch must change at least one property"));
        }

        let schedule = match object.get("schedule") {
            None => None,
            Some(raw) => Some(TypedSchedule::parse(raw)?),
        };
        let action = match object.get("action") {
            None => None,
            Some(raw) => Some(CanonicalAction::parse(raw)?),
        };
        let enabled = match object.get("enabled") {
            None => None,
            Some(raw) => Some(raw.as_bool().ok_or_else(|| {
                invalid("patch.enabled", "enabled must be a boolean when present")
            })?),
        };
        if schedule.is_none() && action.is_none() && enabled.is_none() {
            return Err(invalid(field, "patch must change at least one property"));
        }
        Ok(Self {
            schedule,
            action,
            enabled,
        })
    }

    /// The strongest risk this patch can result in.
    ///
    /// `resulting_contained_action_risk` from the frozen risk rules: a patch
    /// that does not change the action cannot raise the contained risk, so it
    /// carries the operation's own YELLOW floor.
    #[must_use]
    pub fn resulting_risk(&self) -> RiskLevel {
        match &self.action {
            Some(action) => match action.risk() {
                RiskLevel::Green | RiskLevel::Yellow => RiskLevel::Yellow,
                other => other,
            },
            None => RiskLevel::Yellow,
        }
    }

    /// A digest over exactly what this patch changes.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&format!(
            "patch:{}:{}:{}",
            self.schedule
                .as_ref()
                .map(|s| s.canonical())
                .unwrap_or_else(|| "unchanged".to_string()),
            self.action
                .as_ref()
                .map(|a| a.action_digest().as_hex().to_string())
                .unwrap_or_else(|| "unchanged".to_string()),
            self.enabled
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unchanged".to_string()),
        ))
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn action_json(operation_id: &str, params: &[u8]) -> serde_json::Value {
        let tool = operation_id.strip_prefix("os.").unwrap_or(operation_id);
        let schema_digest = frozen_contract(tool)
            .map(input_schema_digest_for)
            .unwrap_or_else(|| Digest::of_str("absent"));
        serde_json::json!({
            "operation_id": operation_id,
            "target": "HostLocalOnly",
            "input_schema_digest": schema_digest.as_hex(),
            "canonical_parameters_cbor":
                base64::engine::general_purpose::STANDARD.encode(params),
            "parameter_digest": Digest::of_bytes(params).as_hex(),
        })
    }

    #[test]
    fn a_shell_command_can_never_be_an_action() {
        // The whole point of the typed action: no representable "run this".
        for hostile in [
            "rm -rf /",
            "os.run_shell_command",
            "/bin/sh",
            "os.exec_command",
        ] {
            let value = action_json(hostile, b"\xa0");
            assert!(
                CanonicalAction::parse(&value).is_err(),
                "{hostile} must be rejected as an action"
            );
        }
    }

    #[test]
    fn an_operation_absent_from_the_frozen_manifest_is_rejected() {
        let value = action_json("os.not_a_real_operation", b"\xa0");
        assert!(CanonicalAction::parse(&value).is_err());
    }

    #[test]
    fn a_known_operation_round_trips_with_bound_digests() {
        let params = b"\xa1\x64test\x01";
        let value = action_json("os.lock_screen", params);
        let action = CanonicalAction::parse(&value).expect("lock_screen is a frozen operation");
        assert_eq!(action.operation_id(), "os.lock_screen");
        assert_eq!(action.parameters_len(), params.len());
        assert_ne!(action.risk(), RiskLevel::Black);
    }

    #[test]
    fn a_mismatched_parameter_digest_is_rejected() {
        let mut value = action_json("os.lock_screen", b"\xa0");
        value["parameter_digest"] = serde_json::json!(Digest::of_str("something else").as_hex());
        assert!(CanonicalAction::parse(&value).is_err());
    }

    #[test]
    fn a_mismatched_schema_digest_is_rejected() {
        let mut value = action_json("os.lock_screen", b"\xa0");
        value["input_schema_digest"] = serde_json::json!(Digest::of_str("stale schema").as_hex());
        assert!(CanonicalAction::parse(&value).is_err());
    }

    #[test]
    fn schedule_bounds_are_enforced_not_clamped() {
        // Below the frozen minimum interval.
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "interval", "every_seconds": 59
        }))
        .is_err());
        // Above the frozen maximum.
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "interval", "every_seconds": 31_536_001u64
        }))
        .is_err());
        // Exactly at the bounds is accepted.
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "interval", "every_seconds": 60
        }))
        .is_ok());
    }

    #[test]
    fn weekly_schedule_rejects_duplicates_and_bad_zones() {
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "weekly", "weekdays": ["monday", "monday"],
            "hour": 9, "minute": 0, "timezone": "Europe/Berlin"
        }))
        .is_err());
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "weekly", "weekdays": ["monday"],
            "hour": 9, "minute": 0, "timezone": "Berlin"
        }))
        .is_err());
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "weekly", "weekdays": ["monday", "friday"],
            "hour": 23, "minute": 59, "timezone": "Europe/Berlin"
        }))
        .is_ok());
    }

    #[test]
    fn an_unknown_schedule_kind_is_an_error_not_a_default() {
        assert!(TypedSchedule::parse(&serde_json::json!({
            "kind": "cron", "expression": "* * * * *"
        }))
        .is_err());
    }

    #[test]
    fn an_empty_patch_is_rejected() {
        assert!(TypedAutomationPatch::parse(&serde_json::json!({})).is_err());
    }

    #[test]
    fn an_unknown_patch_property_is_rejected() {
        assert!(TypedAutomationPatch::parse(&serde_json::json!({
            "command": "curl evil.example"
        }))
        .is_err());
    }

    #[test]
    fn schedule_digest_distinguishes_different_schedules() {
        let a = TypedSchedule::parse(&serde_json::json!({"kind":"once","run_at_ms":1}))
            .expect("valid");
        let b = TypedSchedule::parse(&serde_json::json!({"kind":"once","run_at_ms":2}))
            .expect("valid");
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn an_empty_identifier_is_not_an_identity() {
        assert!(AutomationId::parse("   ").is_err());
        assert!(WorkflowId::parse("").is_err());
        assert_eq!(
            AutomationId::parse("kria-backup.timer")
                .expect("valid")
                .as_str(),
            "kria-backup.timer"
        );
    }
}
