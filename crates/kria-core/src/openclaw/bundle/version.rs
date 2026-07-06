//! Semantic version management (skill-package-contract §5, version manager A2.6).

use semver::{Version, VersionReq};

/// Ordering of a candidate version relative to the currently installed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionRelation {
    /// No prior install.
    Fresh,
    /// Candidate > installed.
    Upgrade,
    /// Candidate == installed.
    Same,
    /// Candidate < installed.
    Downgrade,
}

/// Compare a candidate version against an optionally-installed version.
pub fn relation(candidate: &Version, installed: Option<&Version>) -> VersionRelation {
    match installed {
        None => VersionRelation::Fresh,
        Some(cur) => match candidate.cmp(cur) {
            std::cmp::Ordering::Greater => VersionRelation::Upgrade,
            std::cmp::Ordering::Equal => VersionRelation::Same,
            std::cmp::Ordering::Less => VersionRelation::Downgrade,
        },
    }
}

/// Whether a bundle's `min_kria` is satisfied by the running KRIA version.
pub fn kria_compatible(min_kria: &Version, kria: &Version) -> bool {
    kria >= min_kria
}

/// Whether a dependency requirement string (e.g. "^1.2", ">=1.0, <2.0") is satisfied.
pub fn requirement_satisfied(req: &str, version: &Version) -> Result<bool, String> {
    let vr =
        VersionReq::parse(req).map_err(|e| format!("invalid version requirement '{req}': {e}"))?;
    Ok(vr.matches(version))
}

/// A change is "breaking" when the major version increases (schema/runtime epoch change ⇒
/// reinstall — package-contract §4).
pub fn is_breaking_change(old: &Version, new: &Version) -> bool {
    new.major != old.major
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn relations() {
        assert_eq!(relation(&v("1.0.0"), None), VersionRelation::Fresh);
        assert_eq!(
            relation(&v("1.1.0"), Some(&v("1.0.0"))),
            VersionRelation::Upgrade
        );
        assert_eq!(
            relation(&v("1.0.0"), Some(&v("1.0.0"))),
            VersionRelation::Same
        );
        assert_eq!(
            relation(&v("0.9.0"), Some(&v("1.0.0"))),
            VersionRelation::Downgrade
        );
    }

    #[test]
    fn kria_gate() {
        assert!(kria_compatible(&v("0.1.0"), &v("1.0.0")));
        assert!(!kria_compatible(&v("2.0.0"), &v("1.0.0")));
    }

    #[test]
    fn req_matching() {
        assert!(requirement_satisfied("^1.2", &v("1.4.0")).unwrap());
        assert!(!requirement_satisfied("^1.2", &v("2.0.0")).unwrap());
        assert!(requirement_satisfied(">=1.0, <2.0", &v("1.9.9")).unwrap());
    }

    #[test]
    fn breaking() {
        assert!(is_breaking_change(&v("1.5.0"), &v("2.0.0")));
        assert!(!is_breaking_change(&v("1.5.0"), &v("1.6.0")));
    }
}
