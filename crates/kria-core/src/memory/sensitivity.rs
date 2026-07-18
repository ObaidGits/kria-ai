//! Deterministic sensitivity / PII classifier (memory-upgrade design §47.3, §38.3).
//!
//! Tier-1 detectors only: pattern + structural heuristics, no LLM (LLM refinement
//! is a future slow-path add). Runs on the Write Policy fast path. Fail-safe
//! direction: ambiguity resolves toward *more* private. A `secret` result means
//! the value must never be stored (keychain reference + redaction) and its
//! embedding omitted (§29/N8).

use once_cell::sync::Lazy;
use regex::Regex;

use crate::memory::types::Sensitivity;

/// The result of classification: the assigned class and the detector that drove
/// it (for the audit log / explainability).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensitivityResult {
    pub class: Sensitivity,
    pub detector: Option<&'static str>,
}

macro_rules! re {
    ($s:expr) => {
        Lazy::new(|| Regex::new($s).expect("valid regex"))
    };
}

// ── secret-class detectors ──
static AWS_KEY: Lazy<Regex> = re!(r"\bAKIA[0-9A-Z]{16}\b");
static PRIVATE_KEY: Lazy<Regex> = re!(r"-----BEGIN [A-Z ]*PRIVATE KEY-----");
static JWT: Lazy<Regex> = re!(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+");
static SECRET_ASSIGN: Lazy<Regex> = re!(
    r"(?i)\b(api[_-]?key|secret|password|passwd|token|bearer|client[_-]?secret)\b\s*[:=]\s*\S+"
);
static SSN: Lazy<Regex> = re!(r"\b\d{3}-\d{2}-\d{4}\b");
static CONNECTION_STRING: Lazy<Regex> = re!(r"(?i)\b\w+://[^:\s]+:[^@\s]+@");

// ── private-class detectors ──
static EMAIL: Lazy<Regex> = re!(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b");
static PHONE: Lazy<Regex> = re!(r"(?:\+?\d{1,3}[\s.-]?)?(?:\(?\d{3}\)?[\s.-]?)\d{3}[\s.-]?\d{4}\b");
static MEDICAL_FINANCIAL: Lazy<Regex> = re!(
    r"(?i)\b(diagnos\w*|prescription|medical record|patient|blood pressure|salary|bank account|net worth|iban)\b"
);

/// Luhn checksum — validates candidate card numbers to cut false positives.
fn luhn_valid(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() < 13 || ds.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for &d in ds.iter().rev() {
        let mut v = d;
        if double {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
        double = !double;
    }
    sum % 10 == 0
}

/// Whether `text` contains a Luhn-valid 13–19 digit run (credit card).
fn has_credit_card(text: &str) -> bool {
    let mut run = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else if c == ' ' || c == '-' {
            // keep grouping separators inside a run
        } else {
            if luhn_valid(&run) {
                return true;
            }
            run.clear();
        }
    }
    luhn_valid(&run)
}

/// Classify content sensitivity deterministically (design §47.3).
pub fn classify(content: &str) -> SensitivityResult {
    // secret checks first (highest sensitivity wins).
    let secret_detector = if PRIVATE_KEY.is_match(content) {
        Some("private_key")
    } else if AWS_KEY.is_match(content) {
        Some("aws_key")
    } else if JWT.is_match(content) {
        Some("jwt")
    } else if SECRET_ASSIGN.is_match(content) {
        Some("secret_assignment")
    } else if CONNECTION_STRING.is_match(content) {
        Some("connection_string")
    } else if SSN.is_match(content) {
        Some("ssn")
    } else if has_credit_card(content) {
        Some("credit_card")
    } else {
        None
    };
    if let Some(d) = secret_detector {
        return SensitivityResult {
            class: Sensitivity::Secret,
            detector: Some(d),
        };
    }

    let private_detector = if EMAIL.is_match(content) {
        Some("email")
    } else if MEDICAL_FINANCIAL.is_match(content) {
        Some("medical_financial")
    } else if PHONE.is_match(content) {
        Some("phone")
    } else {
        None
    };
    if let Some(d) = private_detector {
        return SensitivityResult {
            class: Sensitivity::Private,
            detector: Some(d),
        };
    }

    SensitivityResult {
        class: Sensitivity::Public,
        detector: None,
    }
}

/// Combine a deterministic result with an optional caller hint, taking the
/// **more private** of the two (fail-safe, design §47.3).
pub fn resolve(content: &str, hint: Option<&Sensitivity>) -> SensitivityResult {
    let detected = classify(content);
    match hint {
        Some(h) if rank(h) > rank(&detected.class) => SensitivityResult {
            class: h.clone(),
            detector: Some("caller_hint"),
        },
        _ => detected,
    }
}

fn rank(s: &Sensitivity) -> u8 {
    match s {
        Sensitivity::Public => 0,
        Sensitivity::Private => 1,
        Sensitivity::Secret => 2,
        Sensitivity::Other(_) => 1, // unknown → treat as private (fail-safe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secrets() {
        assert_eq!(classify("AKIAIOSFODNN7EXAMPLE").class, Sensitivity::Secret);
        assert_eq!(
            classify("password = hunter2please").class,
            Sensitivity::Secret
        );
        assert_eq!(
            classify("-----BEGIN RSA PRIVATE KEY-----").class,
            Sensitivity::Secret
        );
        assert_eq!(classify("my SSN is 123-45-6789").class, Sensitivity::Secret);
        assert_eq!(
            classify("postgres://user:pass@localhost/db").class,
            Sensitivity::Secret
        );
    }

    #[test]
    fn detects_credit_card_via_luhn() {
        // Valid Visa test number.
        assert_eq!(
            classify("card 4111 1111 1111 1111").class,
            Sensitivity::Secret
        );
        // Random 16 digits that fail Luhn → not flagged as a card.
        assert_ne!(
            classify("order id 1234 5678 9012 3456").detector,
            Some("credit_card")
        );
    }

    #[test]
    fn detects_private_pii() {
        assert_eq!(classify("email me at a@b.com").class, Sensitivity::Private);
        assert_eq!(classify("call 415-555-0132").class, Sensitivity::Private);
        assert_eq!(
            classify("the patient diagnosis was recorded").class,
            Sensitivity::Private
        );
    }

    #[test]
    fn public_by_default() {
        let r = classify("kria runs locally on the laptop");
        assert_eq!(r.class, Sensitivity::Public);
        assert_eq!(r.detector, None);
    }

    #[test]
    fn hint_can_only_raise_sensitivity() {
        // Public content + secret hint → secret (fail-safe upward).
        assert_eq!(
            resolve("hello world", Some(&Sensitivity::Secret)).class,
            Sensitivity::Secret
        );
        // Secret content + public hint → stays secret (never downgraded).
        assert_eq!(
            resolve("AKIAIOSFODNN7EXAMPLE", Some(&Sensitivity::Public)).class,
            Sensitivity::Secret
        );
    }
}
