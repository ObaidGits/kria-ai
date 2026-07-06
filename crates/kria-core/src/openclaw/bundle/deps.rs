//! Dependency resolver (A2.7): skill deps, runtime deps, min_kria, conflict detection.

use super::manifest::Manifest;
use super::version;
use semver::Version;

/// A snapshot of an already-installed skill used for conflict/dependency checks.
#[derive(Debug, Clone)]
pub struct InstalledRef {
    pub slug: String,
    pub version: Version,
    pub publisher: String,
    /// Runtime binaries this skill's substrate provides (used for runtime-dep availability).
    pub provides_runtime: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DepError {
    #[error("requires KRIA >= {required}, but running {running}")]
    KriaTooOld { required: String, running: String },
    #[error("missing skill dependency '{slug}' (requirement '{req}')")]
    MissingSkillDep { slug: String, req: String },
    #[error("skill dependency '{slug}' version {found} does not satisfy '{req}'")]
    SkillDepVersion {
        slug: String,
        found: String,
        req: String,
    },
    #[error("missing runtime dependency '{0}' in substrate")]
    MissingRuntimeDep(String),
    #[error("publisher conflict: '{slug}' already installed from publisher '{existing}', bundle is from '{incoming}'")]
    PublisherConflict {
        slug: String,
        existing: String,
        incoming: String,
    },
    #[error("invalid dependency requirement: {0}")]
    InvalidRequirement(String),
}

/// Runtime capabilities the current substrate is known to provide. A2: the bundled Node image
/// provides `node`; anything else must be resolved into the bundle at package time (deferred).
pub fn substrate_provides() -> Vec<String> {
    vec!["node".to_string()]
}

/// Resolve all dependencies + detect conflicts for `manifest` against the installed set.
pub fn resolve(
    manifest: &Manifest,
    installed: &[InstalledRef],
    kria_version: &Version,
    substrate_runtime: &[String],
) -> Result<(), DepError> {
    // 1. min_kria gate.
    let min_kria = Version::parse(&manifest.skill.min_kria)
        .map_err(|e| DepError::InvalidRequirement(e.to_string()))?;
    if !version::kria_compatible(&min_kria, kria_version) {
        return Err(DepError::KriaTooOld {
            required: min_kria.to_string(),
            running: kria_version.to_string(),
        });
    }

    // 2. Publisher conflict: same slug must keep the same publisher (identity = slug+publisher).
    if let Some(existing) = installed.iter().find(|i| i.slug == manifest.skill.slug) {
        if existing.publisher != manifest.trust.publisher {
            return Err(DepError::PublisherConflict {
                slug: manifest.skill.slug.clone(),
                existing: existing.publisher.clone(),
                incoming: manifest.trust.publisher.clone(),
            });
        }
    }

    // 3. Skill dependencies present + version requirement satisfied.
    for (slug, req) in &manifest.dependencies.skills {
        let Some(dep) = installed.iter().find(|i| &i.slug == slug) else {
            return Err(DepError::MissingSkillDep {
                slug: slug.clone(),
                req: req.clone(),
            });
        };
        let ok = version::requirement_satisfied(req, &dep.version)
            .map_err(DepError::InvalidRequirement)?;
        if !ok {
            return Err(DepError::SkillDepVersion {
                slug: slug.clone(),
                found: dep.version.to_string(),
                req: req.clone(),
            });
        }
    }

    // 4. Runtime dependencies available in the substrate.
    for dep in &manifest.dependencies.runtime {
        if !substrate_runtime.iter().any(|p| p == dep) {
            return Err(DepError::MissingRuntimeDep(dep.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(min_kria: &str) -> Manifest {
        let toml = format!(
            r#"
[skill]
slug = "oc_x"
name = "X"
version = "1.0.0"
category = "productivity"
description = "does x"
min_kria = "{min_kria}"
[runtime]
kind = "docker"
entry = "handler/x.js"
[resource]
class = "light"
[trust]
publisher = "pub-a"
"#
        );
        Manifest::parse(&toml).unwrap()
    }

    #[test]
    fn kria_gate_enforced() {
        let m = manifest("2.0.0");
        let err = resolve(&m, &[], &Version::new(1, 0, 0), &substrate_provides()).unwrap_err();
        assert!(matches!(err, DepError::KriaTooOld { .. }));
    }

    #[test]
    fn publisher_conflict_detected() {
        let m = manifest("0.1.0");
        let installed = vec![InstalledRef {
            slug: "oc_x".into(),
            version: Version::new(0, 9, 0),
            publisher: "pub-b".into(),
            provides_runtime: vec![],
        }];
        let err = resolve(
            &m,
            &installed,
            &Version::new(1, 0, 0),
            &substrate_provides(),
        )
        .unwrap_err();
        assert!(matches!(err, DepError::PublisherConflict { .. }));
    }

    #[test]
    fn missing_runtime_dep() {
        let mut m = manifest("0.1.0");
        m.dependencies.runtime = vec!["ffmpeg".into()];
        let err = resolve(&m, &[], &Version::new(1, 0, 0), &substrate_provides()).unwrap_err();
        assert!(matches!(err, DepError::MissingRuntimeDep(_)));
    }

    #[test]
    fn missing_skill_dep() {
        let mut m = manifest("0.1.0");
        m.dependencies.skills.insert("oc_dep".into(), "^1.0".into());
        let err = resolve(&m, &[], &Version::new(1, 0, 0), &substrate_provides()).unwrap_err();
        assert!(matches!(err, DepError::MissingSkillDep { .. }));
    }

    #[test]
    fn satisfied_deps_pass() {
        let mut m = manifest("0.1.0");
        m.dependencies.skills.insert("oc_dep".into(), "^1.0".into());
        let installed = vec![InstalledRef {
            slug: "oc_dep".into(),
            version: Version::new(1, 3, 0),
            publisher: "any".into(),
            provides_runtime: vec![],
        }];
        assert!(resolve(
            &m,
            &installed,
            &Version::new(1, 0, 0),
            &substrate_provides()
        )
        .is_ok());
    }
}
