//! Capability materialization (A3.2–A3.6): turn `CapabilityGrant`s into an actual container
//! configuration. This is the SINGLE place a container's mounts / network / env / resources /
//! security options are decided — both the warm pool (empty grants → locked default) and the
//! runtime (skill grants) build through here. No hardcoded per-call defaults elsewhere.
//!
//! Deny-by-default: the base config has readonly rootfs, all caps dropped, no-new-privileges,
//! `network=none`, empty env, tmpfs workspace, and resource limits. Grants only *add* the
//! minimum required, and only what was approved.

use super::capability::{CapabilityGrant, Materialization};
use super::types::ResourceClass;
use bollard::container::Config as ContainerConfig;
use bollard::models::{DeviceMapping, HostConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Resource limits materialized onto the container (A3.5). Derived from resource class; the HRA
/// remains the admission authority (resource-contract) — these are the cgroup ceilings.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub memory_bytes: i64,
    pub nano_cpus: i64,
    pub pids_limit: i64,
    pub tmpfs_size: String,
}

impl ResourceLimits {
    pub fn for_class(class: ResourceClass) -> Self {
        match class {
            ResourceClass::Light => Self {
                memory_bytes: 256 * 1024 * 1024,
                nano_cpus: 500_000_000, // 0.5 CPU (nanocpus)
                pids_limit: 128,
                tmpfs_size: "256M".into(),
            },
            ResourceClass::Medium => Self {
                memory_bytes: 512 * 1024 * 1024,
                nano_cpus: 1_000_000_000,
                pids_limit: 256,
                tmpfs_size: "512M".into(),
            },
            ResourceClass::Heavy => Self {
                memory_bytes: 2 * 1024 * 1024 * 1024,
                nano_cpus: 2_000_000_000,
                pids_limit: 512,
                tmpfs_size: "1024M".into(),
            },
        }
    }
}

/// Brokered environment provider — supplies values for allowlisted env var *names* only. Never
/// exposes the host environment (A3.4). Real deployments back this with the secrets vault.
pub trait EnvProvider: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
}

/// Default provider: supplies nothing (empty container env).
pub struct NullEnvProvider;
impl EnvProvider for NullEnvProvider {
    fn get(&self, _name: &str) -> Option<String> {
        None
    }
}

/// The materialized container config + a human/audit summary of what was enforced.
pub struct MaterializedContainer {
    pub config: ContainerConfig<String>,
    pub network_mode: String,
    pub egress_allowlist: Vec<String>,
    pub mounts: Vec<String>,
    pub env_names: Vec<String>,
    pub subprocess_allowlist: Vec<String>,
    pub needs_gpu: bool,
}

/// Build a fully-materialized container config from grants. With `grants = &[]` this yields the
/// locked default profile (the warm-pool base). Every deviation is derived from a grant.
pub fn build(
    image: &str,
    cmd: Vec<String>,
    grants: &[CapabilityGrant],
    limits: &ResourceLimits,
    env_provider: &dyn EnvProvider,
    input_root: Option<&Path>,
) -> MaterializedContainer {
    // ── Deny-by-default base ────────────────────────────────────────────────
    let mut network_mode = "none".to_string();
    let mut egress_allowlist: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    let mut mounts_summary: Vec<String> = Vec::new();
    let mut env: Vec<String> = Vec::new();
    let mut env_names: Vec<String> = Vec::new();
    let mut subprocess_allowlist: Vec<String> = Vec::new();
    let mut devices: Vec<DeviceMapping> = Vec::new();
    let mut needs_gpu = false;

    let mut tmpfs = HashMap::new();
    tmpfs.insert(
        "/workspace".to_string(),
        format!("size={}", limits.tmpfs_size),
    );

    // ── Apply only granted capabilities ─────────────────────────────────────
    for grant in grants {
        if !grant.granted {
            continue;
        }
        match &grant.materialization {
            Materialization::WorkspaceTmpfs => { /* always present */ }
            Materialization::InputMount { id, read_only } => {
                let host = resolve_input(input_root, id);
                let container_path = format!("/inputs/{id}");
                let mode = if *read_only { "ro" } else { "rw" };
                binds.push(format!("{}:{}:{}", host.display(), container_path, mode));
                mounts_summary.push(format!("{} ({})", container_path, mode));
            }
            Materialization::EgressAllowlist(domains) => {
                // Network enabled only because a grant asked for it; else it stays `none`.
                network_mode = "bridge".to_string();
                egress_allowlist.extend(domains.iter().cloned());
            }
            Materialization::EnvAllowlist(names) => {
                for name in names {
                    if let Some(val) = env_provider.get(name) {
                        env.push(format!("{name}={val}"));
                    }
                    env_names.push(name.clone());
                }
            }
            Materialization::SubprocessAllowlist(bins) => {
                subprocess_allowlist.extend(bins.iter().cloned());
            }
            Materialization::Device(dev) => {
                if !dev.is_empty() {
                    devices.push(DeviceMapping {
                        path_on_host: Some(dev.clone()),
                        path_in_container: Some(dev.clone()),
                        cgroup_permissions: Some("rwm".to_string()),
                    });
                }
            }
            Materialization::GpuLease => {
                needs_gpu = true;
            }
            Materialization::BrokeredBrowser | Materialization::None => {}
        }
    }

    let host_config = HostConfig {
        memory: Some(limits.memory_bytes),
        nano_cpus: Some(limits.nano_cpus),
        pids_limit: Some(limits.pids_limit),
        readonly_rootfs: Some(true),
        network_mode: Some(network_mode.clone()),
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        cap_drop: Some(vec!["ALL".to_string()]),
        tmpfs: Some(tmpfs),
        binds: if binds.is_empty() { None } else { Some(binds) },
        devices: if devices.is_empty() {
            None
        } else {
            Some(devices)
        },
        ..Default::default()
    };

    let mut labels = HashMap::new();
    labels.insert("ai.kria.component".to_string(), "openclaw".to_string());
    labels.insert("ai.kria.managed".to_string(), "true".to_string());

    let config = ContainerConfig {
        image: Some(image.to_string()),
        cmd: Some(cmd),
        env: if env.is_empty() { None } else { Some(env) },
        host_config: Some(host_config),
        labels: Some(labels),
        ..Default::default()
    };

    MaterializedContainer {
        config,
        network_mode,
        egress_allowlist,
        mounts: mounts_summary,
        env_names,
        subprocess_allowlist,
        needs_gpu,
    }
}

fn resolve_input(input_root: Option<&Path>, id: &str) -> PathBuf {
    let p = Path::new(id);
    if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(root) = input_root {
        root.join(id)
    } else {
        PathBuf::from(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{
        grant_all, Capability, CapabilityKind, CapabilityMode, CapabilityScope, GrantSource,
    };

    fn limits() -> ResourceLimits {
        ResourceLimits::for_class(ResourceClass::Light)
    }

    struct MapEnv(std::collections::HashMap<String, String>);
    impl EnvProvider for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn cap(kind: CapabilityKind, mode: CapabilityMode, scope: CapabilityScope) -> Capability {
        Capability { kind, mode, scope }
    }

    #[test]
    fn default_profile_is_locked() {
        let m = build(
            "img",
            vec!["node".into()],
            &[],
            &limits(),
            &NullEnvProvider,
            None,
        );
        let hc = m.config.host_config.unwrap();
        assert_eq!(hc.network_mode.as_deref(), Some("none"));
        assert_eq!(hc.readonly_rootfs, Some(true));
        assert_eq!(hc.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(hc.pids_limit, Some(128));
        assert!(hc.binds.is_none(), "no blanket mounts");
        assert!(m.config.env.is_none(), "no host env");
        assert_eq!(m.network_mode, "none");
    }

    #[test]
    fn filesystem_grant_adds_only_scoped_mount() {
        let caps = vec![cap(
            CapabilityKind::Filesystem,
            CapabilityMode::ReadOnly,
            CapabilityScope::InputMount("docs".into()),
        )];
        let grants = grant_all(&caps, GrantSource::UserApproval, true);
        let m = build(
            "img",
            vec!["node".into()],
            &grants,
            &limits(),
            &NullEnvProvider,
            Some(Path::new("/host/inputs")),
        );
        let hc = m.config.host_config.unwrap();
        let binds = hc.binds.unwrap();
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0], "/host/inputs/docs:/inputs/docs:ro");
    }

    #[test]
    fn network_none_unless_granted() {
        // No network grant → none.
        let m = build(
            "img",
            vec!["node".into()],
            &[],
            &limits(),
            &NullEnvProvider,
            None,
        );
        assert_eq!(m.network_mode, "none");
        // With allowlist grant → enabled + recorded.
        let caps = vec![cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["api.example.com".into()]),
        )];
        let grants = grant_all(&caps, GrantSource::UserApproval, true);
        let m2 = build(
            "img",
            vec!["node".into()],
            &grants,
            &limits(),
            &NullEnvProvider,
            None,
        );
        assert_eq!(m2.network_mode, "bridge");
        assert_eq!(m2.egress_allowlist, vec!["api.example.com".to_string()]);
    }

    #[test]
    fn env_only_allowlisted_from_broker() {
        let caps = vec![cap(
            CapabilityKind::Environment,
            CapabilityMode::Use,
            CapabilityScope::EnvVars(vec!["API_KEY".into(), "MISSING".into()]),
        )];
        let grants = grant_all(&caps, GrantSource::UserApproval, true);
        let mut map = std::collections::HashMap::new();
        map.insert("API_KEY".to_string(), "secret123".to_string());
        let m = build(
            "img",
            vec!["node".into()],
            &grants,
            &limits(),
            &MapEnv(map),
            None,
        );
        let env = m.config.env.unwrap();
        // Only the provided allowlisted var is injected; MISSING has no value → not present.
        assert_eq!(env, vec!["API_KEY=secret123".to_string()]);
        assert_eq!(
            m.env_names,
            vec!["API_KEY".to_string(), "MISSING".to_string()]
        );
    }

    #[test]
    fn ungranted_capability_is_not_materialized() {
        let caps = vec![cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["x.com".into()]),
        )];
        // granted = false → must NOT open network.
        let grants = grant_all(&caps, GrantSource::Manifest, false);
        let m = build(
            "img",
            vec!["node".into()],
            &grants,
            &limits(),
            &NullEnvProvider,
            None,
        );
        assert_eq!(m.network_mode, "none");
        assert!(m.egress_allowlist.is_empty());
    }

    #[test]
    fn gpu_grant_flags_needs_gpu() {
        let caps = vec![cap(
            CapabilityKind::Gpu,
            CapabilityMode::Use,
            CapabilityScope::None,
        )];
        let grants = grant_all(&caps, GrantSource::UserApproval, true);
        let m = build(
            "img",
            vec!["node".into()],
            &grants,
            &limits(),
            &NullEnvProvider,
            None,
        );
        assert!(m.needs_gpu);
    }
}
