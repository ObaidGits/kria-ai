//! Variable Safety — Strict validation for extracted variables.
//!
//! # Design Principle: Defense in Depth
//!
//! Even if the LLM generates a "valid" variable value, we validate it again
//! at the Rust level before it reaches the command execution layer.
//!
//! Validation layers:
//! 1. Type-specific regex (e.g., IP must match IPv4/IPv6 pattern)
//! 2. Shell metacharacter rejection (no ;|&`$(){}<>\)
//! 3. Length limits (prevent buffer overflow scenarios)
//! 4. Allowlist for known-dangerous types (FilePath must start with /)

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::VariableType;

/// Validation error for a variable value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Value contains shell metacharacters.
    ContainsMetacharacters { value: String, chars: String },
    /// Value doesn't match the type's regex pattern.
    TypeMismatch { expected_type: VariableType, value: String },
    /// Value is too long.
    TooLong { max_len: usize, actual_len: usize },
    /// Value is empty.
    Empty,
    /// Value contains null bytes.
    ContainsNullByte,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContainsMetacharacters { value, chars } => {
                write!(f, "value '{}' contains forbidden characters: {}", value, chars)
            }
            Self::TypeMismatch { expected_type, value } => {
                write!(f, "value '{}' does not match type {:?}", value, expected_type)
            }
            Self::TooLong { max_len, actual_len } => {
                write!(f, "value too long: {} > {}", actual_len, max_len)
            }
            Self::Empty => write!(f, "value is empty"),
            Self::ContainsNullByte => write!(f, "value contains null byte"),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Maximum length for any variable value.
const MAX_VARIABLE_LENGTH: usize = 1024;

/// Shell metacharacters that are NEVER allowed in variable values.
static SHELL_METACHARACTERS: Lazy<Vec<char>> = Lazy::new(|| {
    vec![';', '|', '&', '`', '$', '(', ')', '{', '}', '<', '>', '\\', '\n', '\r']
});

/// Validate a variable value against its declared type.
///
/// # Validation Order
///
/// 1. Empty check
/// 2. Null byte check
/// 3. Length check
/// 4. Shell metacharacter check (ALL types)
/// 5. Type-specific regex check
pub fn validate_variable(value: &str, var_type: &VariableType) -> Result<(), ValidationError> {
    // 1. Empty
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }

    // 2. Null bytes
    if value.contains('\0') {
        return Err(ValidationError::ContainsNullByte);
    }

    // 3. Length
    if value.len() > MAX_VARIABLE_LENGTH {
        return Err(ValidationError::TooLong {
            max_len: MAX_VARIABLE_LENGTH,
            actual_len: value.len(),
        });
    }

    // 4. Shell metacharacters (ALL types reject these)
    let forbidden: String = SHELL_METACHARACTERS.iter()
        .filter(|c| value.contains(**c))
        .collect();
    if !forbidden.is_empty() {
        return Err(ValidationError::ContainsMetacharacters {
            value: value.to_string(),
            chars: forbidden,
        });
    }

    // 5. Type-specific regex
    static REGEX_CACHE: Lazy<std::sync::Mutex<std::collections::HashMap<String, Regex>>> =
        Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

    let pattern = var_type.validation_pattern();
    let regex = {
        let mut cache = REGEX_CACHE.lock().unwrap();
        cache.entry(pattern.to_string())
            .or_insert_with(|| Regex::new(pattern).unwrap())
            .clone()
    };

    if !regex.is_match(value) {
        return Err(ValidationError::TypeMismatch {
            expected_type: var_type.clone(),
            value: value.to_string(),
        });
    }

    Ok(())
}

/// Infer the VariableType from a concrete value.
pub fn infer_variable_type(value: &str) -> VariableType {
    // Check in order of specificity
    static IP_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$").unwrap()
    });
    static PORT_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^\d{1,5}$").unwrap()
    });
    static PATH_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^/[\w./\-]+$").unwrap()
    });
    static NUMERIC_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^\d+\.?\d*$").unwrap()
    });
    static SERVICE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[a-zA-Z][\w\-\.]*$").unwrap()
    });
    static HOSTNAME_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^[a-zA-Z0-9][\w\-\.]+\.[a-zA-Z]{2,}$").unwrap()
    });

    if IP_RE.is_match(value) {
        VariableType::IpAddress
    } else if PORT_RE.is_match(value) {
        if let Ok(port) = value.parse::<u16>() {
            if port > 0 {
                return VariableType::PortNumber;
            }
        }
        VariableType::Numeric
    } else if PATH_RE.is_match(value) {
        VariableType::FilePath
    } else if HOSTNAME_RE.is_match(value) {
        VariableType::Hostname
    } else if NUMERIC_RE.is_match(value) {
        VariableType::Numeric
    } else if SERVICE_RE.is_match(value) {
        VariableType::ServiceName
    } else {
        VariableType::String
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ip_address() {
        assert!(validate_variable("192.168.1.1", &VariableType::IpAddress).is_ok());
        assert!(validate_variable("10.0.0.1", &VariableType::IpAddress).is_ok());
        assert!(validate_variable("not-an-ip", &VariableType::IpAddress).is_err());
    }

    #[test]
    fn validate_file_path() {
        assert!(validate_variable("/etc/nginx/nginx.conf", &VariableType::FilePath).is_ok());
        assert!(validate_variable("/tmp/test.txt", &VariableType::FilePath).is_ok());
        assert!(validate_variable("relative/path", &VariableType::FilePath).is_err());
    }

    #[test]
    fn validate_service_name() {
        assert!(validate_variable("nginx", &VariableType::ServiceName).is_ok());
        assert!(validate_variable("my-service", &VariableType::ServiceName).is_ok());
        assert!(validate_variable("my.service", &VariableType::ServiceName).is_ok());
    }

    #[test]
    fn validate_port_number() {
        assert!(validate_variable("80", &VariableType::PortNumber).is_ok());
        assert!(validate_variable("8080", &VariableType::PortNumber).is_ok());
        assert!(validate_variable("65535", &VariableType::PortNumber).is_ok());
        assert!(validate_variable("0", &VariableType::PortNumber).is_err()); // 0 is not a valid port (regex rejects leading zero)
    }

    #[test]
    fn reject_shell_metacharacters() {
        assert!(validate_variable("test;rm -rf /", &VariableType::String).is_err());
        assert!(validate_variable("test|cat /etc/passwd", &VariableType::String).is_err());
        assert!(validate_variable("test`whoami`", &VariableType::String).is_err());
        assert!(validate_variable("test$(whoami)", &VariableType::String).is_err());
    }

    #[test]
    fn reject_empty() {
        assert!(validate_variable("", &VariableType::String).is_err());
    }

    #[test]
    fn reject_null_byte() {
        assert!(validate_variable("test\0evil", &VariableType::String).is_err());
    }

    #[test]
    fn infer_ip() {
        assert_eq!(infer_variable_type("192.168.1.1"), VariableType::IpAddress);
    }

    #[test]
    fn infer_path() {
        assert_eq!(infer_variable_type("/etc/nginx.conf"), VariableType::FilePath);
    }

    #[test]
    fn infer_port() {
        assert_eq!(infer_variable_type("8080"), VariableType::PortNumber);
    }

    #[test]
    fn infer_hostname() {
        assert_eq!(infer_variable_type("example.com"), VariableType::Hostname);
    }

    #[test]
    fn infer_service() {
        assert_eq!(infer_variable_type("nginx"), VariableType::ServiceName);
    }
}
