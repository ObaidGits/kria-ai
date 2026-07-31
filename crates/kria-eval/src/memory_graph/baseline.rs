//! F0.5 baseline capture scaffolding: the **Reference Hardware ID**, the
//! environment-capture harness, and the **warm-up + sample protocol** for the
//! Memory Graph Production Redesign spec (task F0.5 / 0.5.1).
//!
//! `validation.md` §3 requires every evidence `manifest.json` to pin a
//! *reference-hardware ID (CPU/RAM/GPU/storage/display/DPI)*, the
//! *OS/kernel/WebKitGTK/runtime/build profile*, *power/thermal/network state*,
//! the *warm/cold protocol*, and *locale/theme/input/AT*. `V-PERF-01` fixes the
//! measurement protocol shape: *"≥30 warm iterations plus separate cold"* with
//! percentile reporting. This module renders those two obligations as:
//!
//! * a **stable, deterministic** [`ReferenceHardwareId`] derived purely from a
//!   machine's descriptive identity ([`HardwareFingerprint`]), so the same
//!   hardware always produces the same ID and a hardware change produces a
//!   different one; and
//! * a [`BaselineEnvironment::capture`] harness that fills the manifest's
//!   [`ReferenceHardware`] / [`BuildEnvironment`] / [`EnvironmentState`] /
//!   [`Accessibility`] structs from *cheaply detectable* sources (`/proc`,
//!   `/sys`, `std::env`, `cfg!`), recording an explicit [`UNKNOWN`] marker for
//!   required facts it cannot resolve and `None` for optional ones — it never
//!   fabricates a plausible-looking value; and
//! * the [`SampleProtocol`] definition: warm-up count, sampled-iteration count,
//!   the collected percentile set (p50/p95/p99), and the cold-vs-warm
//!   distinction tied to [`MeasurementProtocol`].
//!
//! ## Scope boundary (0.5.1 only)
//!
//! This task defines the **ID scheme, the capture harness, and the protocol
//! constants**. It deliberately does **not** run the latency/CPU/RAM/frame
//! baseline measurements or take screenshots — that is task **0.5.2**; it does
//! not run the ID/fixture/manifest commands or resolve orphans (**0.5.3**); and
//! it does not generate or sign the F0 manifest (**0.5.4**). The
//! [`SampleProtocol::summarize`] percentile helper is scaffolding the later
//! measurement task consumes; here it is only defined and unit-tested.

use sha2::{Digest, Sha256};

use super::fixtures::hex_lower;
use super::manifest::{
    Accessibility, BuildEnvironment, EnvironmentState, MeasurementProtocol, ReferenceHardware,
};

/// Explicit marker recorded for a **required** environment fact that could not
/// be cheaply resolved on the capture host. Using a sentinel (rather than a
/// fabricated value) keeps the spec invariant that unresolved facts are
/// recorded `Unknown`/`Unavailable`, never inferred (`tasks.md` F0.5:
/// "unresolved facts are recorded `Unknown`/`Unavailable`, never inferred").
pub const UNKNOWN: &str = "Unknown";

/// Stable prefix on every derived [`ReferenceHardwareId`].
pub const HARDWARE_ID_PREFIX: &str = "mg-ref-hw";

/// Number of hex characters of the fingerprint digest carried in a derived ID.
const HARDWARE_ID_HEX_LEN: usize = 16;

// --- Warm-up + sample protocol constants (V-PERF-01) ---------------------

/// Warm-up iterations discarded before sampling begins (prime caches/JITs so
/// the sampled window measures warm steady-state behavior).
pub const WARMUP_ITERATIONS: usize = 5;

/// Sampled iterations retained for statistics. `V-PERF-01` mandates
/// *"≥30 warm iterations"*; this is the floor.
pub const SAMPLE_ITERATIONS: usize = 30;

/// The percentile set collected for every latency metric (p50/p95/p99).
pub const PERCENTILES: [f64; 3] = [50.0, 95.0, 99.0];

/// The descriptive, *identity-bearing* subset of a machine's hardware/build
/// profile. This is what the [`ReferenceHardwareId`] is derived from, so it
/// deliberately excludes volatile runtime state (power/thermal/network) and the
/// ID itself — only stable identity fields participate, so the ID is invariant
/// across runs on the same machine but changes when the machine changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareFingerprint {
    /// CPU description.
    pub cpu: String,
    /// RAM description.
    pub ram: String,
    /// GPU description (or a marker when absent/undetected).
    pub gpu: String,
    /// Storage description.
    pub storage: String,
    /// Display description.
    pub display: String,
    /// DPI/scale description.
    pub dpi: String,
    /// Operating system name/version.
    pub os: String,
    /// Kernel version.
    pub kernel: String,
    /// Language runtime/toolchain.
    pub runtime: String,
    /// Build profile (`debug`/`release`).
    pub build_profile: String,
}

impl HardwareFingerprint {
    /// Canonical, order-stable serialization used as the digest preimage. Each
    /// field is emitted on its own `key=value` line so a change in any single
    /// field changes the digest, and the encoding is unambiguous (no field can
    /// masquerade as another).
    fn canonical(&self) -> String {
        format!(
            "cpu={}\nram={}\ngpu={}\nstorage={}\ndisplay={}\ndpi={}\nos={}\nkernel={}\nruntime={}\nbuildProfile={}\n",
            self.cpu,
            self.ram,
            self.gpu,
            self.storage,
            self.display,
            self.dpi,
            self.os,
            self.kernel,
            self.runtime,
            self.build_profile,
        )
    }

    /// Derive the stable [`ReferenceHardwareId`] for this fingerprint.
    pub fn id(&self) -> ReferenceHardwareId {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical().as_bytes());
        let digest = hex_lower(&hasher.finalize());
        ReferenceHardwareId(format!(
            "{HARDWARE_ID_PREFIX}-{}",
            &digest[..HARDWARE_ID_HEX_LEN]
        ))
    }
}

/// A stable, deterministic reference-hardware identifier.
///
/// Deriving the ID from a [`HardwareFingerprint`] is a pure function: identical
/// descriptions always yield an identical ID (`==`), and any change to a
/// descriptive field yields a different ID. The string form is
/// `mg-ref-hw-<16 hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceHardwareId(String);

impl ReferenceHardwareId {
    /// Borrow the identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned identifier string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for ReferenceHardwareId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The four manifest environment structs captured together for an F0 baseline
/// run, plus the derived reference-hardware ID.
///
/// These are exactly the manifest field types, so a captured environment drops
/// straight into an [`EvidenceManifest`](super::manifest::EvidenceManifest)
/// without translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEnvironment {
    /// Reference-hardware identity (`hardware_id` is the derived stable ID).
    pub reference_hardware: ReferenceHardware,
    /// OS/kernel/WebKitGTK/runtime/build profile.
    pub build_environment: BuildEnvironment,
    /// Power/thermal/network state and the warm/cold protocol.
    pub environment_state: EnvironmentState,
    /// Locale/theme/input/AT.
    pub accessibility: Accessibility,
}

impl BaselineEnvironment {
    /// Capture the current environment from cheaply detectable sources.
    ///
    /// Detection is best-effort and dependency-free (reads `/proc`, `/sys`,
    /// `std::env`, and `cfg!`). Required manifest fields that cannot be resolved
    /// are recorded as the explicit [`UNKNOWN`] marker; optional fields that
    /// cannot be resolved are recorded as `None`. Nothing is fabricated.
    ///
    /// The `protocol` argument records whether this baseline slice measures the
    /// warm phase, the cold phase, or both (`validation.md` warm/cold protocol).
    pub fn capture(protocol: MeasurementProtocol) -> Self {
        // --- identity-bearing fields (feed the fingerprint) ---
        let cpu = detect_cpu().unwrap_or_else(|| UNKNOWN.to_string());
        let ram = detect_ram().unwrap_or_else(|| UNKNOWN.to_string());
        let os = detect_os();
        let kernel = detect_kernel().unwrap_or_else(|| UNKNOWN.to_string());
        let runtime = detect_runtime().unwrap_or_else(|| UNKNOWN.to_string());
        let build_profile = detect_build_profile().to_string();

        // Optional identity fields: not cheaply/headlessly detectable ->
        // recorded as absent (None), never fabricated. The fingerprint folds in
        // the UNKNOWN marker for these so the ID stays deterministic.
        let gpu: Option<String> = None;
        let storage: Option<String> = None;
        let display: Option<String> = None;
        let dpi: Option<String> = None;

        let fingerprint = HardwareFingerprint {
            cpu: cpu.clone(),
            ram: ram.clone(),
            gpu: gpu.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            storage: storage.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            display: display.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            dpi: dpi.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            os: os.clone(),
            kernel: kernel.clone(),
            runtime: runtime.clone(),
            build_profile: build_profile.clone(),
        };
        let hardware_id = fingerprint.id().into_string();

        let reference_hardware = ReferenceHardware {
            hardware_id: Some(hardware_id),
            cpu: Some(cpu),
            ram: Some(ram),
            gpu,
            storage,
            display,
            dpi,
        };

        let build_environment = BuildEnvironment {
            os: Some(os),
            kernel: Some(kernel),
            // WebKitGTK is meaningful only for GUI runs and is not cheaply
            // detectable from a headless backend capture -> absent.
            webkit_gtk: None,
            runtime: Some(runtime),
            build_profile: Some(build_profile),
            // Lockfile/binary hashes are collected by the manifest builder
            // (0.5.4), not by environment capture.
            lockfile_hashes: Default::default(),
            binary_hashes: Default::default(),
        };

        let environment_state = EnvironmentState {
            power_state: Some(detect_power_state().unwrap_or_else(|| UNKNOWN.to_string())),
            // Thermal state is optional and not reliably comparable across
            // machines -> absent unless a future task opts in.
            thermal_state: None,
            network_state: Some(detect_network_state().unwrap_or_else(|| UNKNOWN.to_string())),
            protocol,
        };

        let accessibility = Accessibility {
            locale: Some(detect_locale().unwrap_or_else(|| UNKNOWN.to_string())),
            theme: detect_theme(),
            // Input modality and assistive technology are not detectable from a
            // headless capture -> absent (optional).
            input: None,
            assistive_tech: None,
        };

        BaselineEnvironment {
            reference_hardware,
            build_environment,
            environment_state,
            accessibility,
        }
    }
}

/// The warm-up + sampling protocol for latency baselines.
///
/// This is the *definition* used by task 0.5.2's actual measurement run: how
/// many warm-up iterations to discard, how many to sample, which percentiles to
/// report, and whether the slice is warm/cold/both. [`SampleProtocol::default`]
/// yields the `V-PERF-01`-compliant protocol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleProtocol {
    /// Warm-up iterations discarded before sampling.
    pub warmup_iterations: usize,
    /// Sampled iterations retained for statistics.
    pub sample_iterations: usize,
    /// Percentiles collected for each metric.
    pub percentiles: [f64; 3],
    /// Whether this slice measures the warm phase, cold phase, or both.
    pub protocol: MeasurementProtocol,
}

impl Default for SampleProtocol {
    fn default() -> Self {
        SampleProtocol {
            warmup_iterations: WARMUP_ITERATIONS,
            sample_iterations: SAMPLE_ITERATIONS,
            percentiles: PERCENTILES,
            protocol: MeasurementProtocol::WarmAndCold,
        }
    }
}

/// The percentile summary a latency metric reports under the protocol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PercentileSummary {
    /// Median (p50).
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
}

impl SampleProtocol {
    /// Summarize a set of raw latency samples into the collected percentile set.
    ///
    /// Uses the nearest-rank method on a sorted copy of `samples`. Returns
    /// `None` for an empty input (a metric with no samples has no percentiles;
    /// the *`Unavailable`* recording of that case is the caller's job). This is
    /// pure scaffolding the 0.5.2 measurement task consumes.
    pub fn summarize(&self, samples: &[f64]) -> Option<PercentileSummary> {
        Some(PercentileSummary {
            p50: percentile(samples, 50.0)?,
            p95: percentile(samples, 95.0)?,
            p99: percentile(samples, 99.0)?,
        })
    }
}

/// Nearest-rank percentile of `samples` for `p` in `[0, 100]`. `None` on empty.
fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    // Nearest-rank: rank = ceil(p/100 * n), clamped to [1, n].
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    Some(sorted[idx])
}

// --- Cheap, dependency-free detectors ------------------------------------

/// CPU model from `/proc/cpuinfo` (`model name`), `None` when unavailable.
fn detect_cpu() -> Option<String> {
    let contents = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in contents.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if key == "model name" || key == "Model" || key == "Hardware" {
                let v = value.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Total RAM from `/proc/meminfo` (`MemTotal`), rendered as `NN.N GiB`.
fn detect_ram() -> Option<String> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
            let gib = kb as f64 / (1024.0 * 1024.0);
            return Some(format!("{gib:.1} GiB"));
        }
    }
    None
}

/// OS description: `PRETTY_NAME` from `/etc/os-release`, else the compile-time
/// target OS constant (always available, so this never falls back to UNKNOWN).
fn detect_os() -> String {
    if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                let v = rest.trim().trim_matches('"').trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

/// Kernel release from `/proc/sys/kernel/osrelease`, `None` when unavailable.
fn detect_kernel() -> Option<String> {
    let v = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Rust toolchain: prefers the `RUSTUP_TOOLCHAIN` env var, then a compile-time
/// pinned version if the build injected one; `None` when neither is present.
fn detect_runtime() -> Option<String> {
    if let Ok(tc) = std::env::var("RUSTUP_TOOLCHAIN") {
        let tc = tc.trim();
        if !tc.is_empty() {
            return Some(format!("rustup {tc}"));
        }
    }
    option_env!("CARGO_PKG_RUST_VERSION").map(|v| format!("rustc (pinned {v})"))
}

/// Build profile from the compile-time `debug_assertions` cfg. Always resolves.
fn detect_build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// AC/power state from `/sys/class/power_supply/*/online`, `None` when absent.
fn detect_power_state() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let mut type_path = entry.path();
        type_path.push("type");
        let kind = std::fs::read_to_string(&type_path).unwrap_or_default();
        if kind.trim() == "Mains" {
            let mut online_path = entry.path();
            online_path.push("online");
            if let Ok(online) = std::fs::read_to_string(&online_path) {
                return Some(if online.trim() == "1" {
                    "AC".to_string()
                } else {
                    "battery".to_string()
                });
            }
        }
    }
    None
}

/// Network reachability is not cheaply/deterministically detectable without a
/// probe (out of 0.5.1 scope) -> always `None` (recorded as UNKNOWN upstream).
fn detect_network_state() -> Option<String> {
    None
}

/// Locale from the standard `LC_ALL`/`LANG` env vars, `None` when unset.
fn detect_locale() -> Option<String> {
    for var in ["LC_ALL", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Theme hint from the `GTK_THEME` env var, `None` when unset (optional field).
fn detect_theme() -> Option<String> {
    let v = std::env::var("GTK_THEME").ok()?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fingerprint() -> HardwareFingerprint {
        HardwareFingerprint {
            cpu: "AMD Ryzen 7 5800H".to_string(),
            ram: "32.0 GiB".to_string(),
            gpu: UNKNOWN.to_string(),
            storage: UNKNOWN.to_string(),
            display: UNKNOWN.to_string(),
            dpi: UNKNOWN.to_string(),
            os: "Ubuntu 24.04".to_string(),
            kernel: "6.8.0-generic".to_string(),
            runtime: "rustc 1.83".to_string(),
            build_profile: "release".to_string(),
        }
    }

    #[test]
    fn hardware_id_is_deterministic_for_a_given_description() {
        let fp = sample_fingerprint();
        let a = fp.id();
        let b = fp.clone().id();
        assert_eq!(a, b, "same description must yield the same ID");
        // Stable format contract.
        assert!(a.as_str().starts_with(HARDWARE_ID_PREFIX));
        assert_eq!(
            a.as_str().len(),
            HARDWARE_ID_PREFIX.len() + 1 + HARDWARE_ID_HEX_LEN
        );
    }

    #[test]
    fn hardware_id_changes_when_any_identity_field_changes() {
        let base = sample_fingerprint().id();
        // Each single-field mutation must produce a different ID.
        let mutations: Vec<Box<dyn Fn(&mut HardwareFingerprint)>> = vec![
            Box::new(|f| f.cpu = "Intel i7".to_string()),
            Box::new(|f| f.ram = "16.0 GiB".to_string()),
            Box::new(|f| f.gpu = "NVIDIA".to_string()),
            Box::new(|f| f.storage = "NVMe".to_string()),
            Box::new(|f| f.display = "1920x1080".to_string()),
            Box::new(|f| f.dpi = "192".to_string()),
            Box::new(|f| f.os = "Fedora 40".to_string()),
            Box::new(|f| f.kernel = "6.9.0".to_string()),
            Box::new(|f| f.runtime = "rustc 1.84".to_string()),
            Box::new(|f| f.build_profile = "debug".to_string()),
        ];
        for mutate in mutations {
            let mut fp = sample_fingerprint();
            mutate(&mut fp);
            assert_ne!(
                base,
                fp.id(),
                "mutating an identity field must change the ID"
            );
        }
    }

    #[test]
    fn capture_populates_manifest_structs_without_fabricating() {
        let env = BaselineEnvironment::capture(MeasurementProtocol::WarmAndCold);

        // Required-for-manifest fields are always present (real value or the
        // explicit UNKNOWN marker), never null.
        let hw = &env.reference_hardware;
        assert!(hw
            .hardware_id
            .as_deref()
            .unwrap()
            .starts_with(HARDWARE_ID_PREFIX));
        assert!(hw.cpu.is_some());
        assert!(hw.ram.is_some());
        assert!(env.build_environment.os.is_some());
        assert!(env.build_environment.kernel.is_some());
        assert!(env.build_environment.runtime.is_some());
        assert!(env.build_environment.build_profile.is_some());
        assert!(env.environment_state.power_state.is_some());
        assert!(env.environment_state.network_state.is_some());
        assert!(env.accessibility.locale.is_some());

        // Undetectable optional fields are recorded as absent (None), NOT
        // fabricated with a plausible value.
        assert_eq!(hw.gpu, None);
        assert_eq!(hw.storage, None);
        assert_eq!(hw.display, None);
        assert_eq!(hw.dpi, None);
        assert_eq!(env.build_environment.webkit_gtk, None);
        assert_eq!(env.environment_state.thermal_state, None);
        assert_eq!(env.accessibility.input, None);
        assert_eq!(env.accessibility.assistive_tech, None);

        // build_profile reflects the compile-time cfg deterministically.
        let profile = env.build_environment.build_profile.as_deref().unwrap();
        assert!(profile == "debug" || profile == "release");

        // network_state is not probed in 0.5.1 -> recorded as the UNKNOWN marker.
        assert_eq!(
            env.environment_state.network_state.as_deref(),
            Some(UNKNOWN)
        );

        // The recorded protocol round-trips.
        assert_eq!(
            env.environment_state.protocol,
            MeasurementProtocol::WarmAndCold
        );
    }

    #[test]
    fn captured_hardware_id_matches_its_fingerprint() {
        // The captured ID must equal the ID derived from the captured
        // descriptive fields (i.e. capture uses the same deterministic scheme).
        let env = BaselineEnvironment::capture(MeasurementProtocol::Warm);
        let hw = &env.reference_hardware;
        let fp = HardwareFingerprint {
            cpu: hw.cpu.clone().unwrap(),
            ram: hw.ram.clone().unwrap(),
            gpu: hw.gpu.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            storage: hw.storage.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            display: hw.display.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            dpi: hw.dpi.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            os: env.build_environment.os.clone().unwrap(),
            kernel: env.build_environment.kernel.clone().unwrap(),
            runtime: env.build_environment.runtime.clone().unwrap(),
            build_profile: env.build_environment.build_profile.clone().unwrap(),
        };
        assert_eq!(hw.hardware_id.as_deref().unwrap(), fp.id().as_str());
    }

    #[test]
    fn sample_protocol_exposes_warmup_sample_and_percentiles() {
        let p = SampleProtocol::default();
        assert_eq!(p.warmup_iterations, WARMUP_ITERATIONS);
        assert_eq!(p.sample_iterations, SAMPLE_ITERATIONS);
        assert!(
            p.sample_iterations >= 30,
            "V-PERF-01 requires >=30 warm iterations"
        );
        assert_eq!(p.percentiles, [50.0, 95.0, 99.0]);
        assert_eq!(p.protocol, MeasurementProtocol::WarmAndCold);
    }

    #[test]
    fn percentile_summary_is_correct_and_empty_is_none() {
        let p = SampleProtocol::default();
        // 1..=100 ms samples: nearest-rank p50=50, p95=95, p99=99.
        let samples: Vec<f64> = (1..=100).map(|n| n as f64).collect();
        let summary = p.summarize(&samples).expect("non-empty samples summarize");
        assert_eq!(summary.p50, 50.0);
        assert_eq!(summary.p95, 95.0);
        assert_eq!(summary.p99, 99.0);
        // Empty input -> no percentiles (caller records Unavailable).
        assert_eq!(p.summarize(&[]), None);
    }

    #[test]
    fn captured_environment_round_trips_into_manifest_types() {
        // The captured EnvironmentState/Accessibility (and the other two
        // structs) are the manifest field types, so they must serialize and
        // deserialize losslessly via serde_json.
        let env = BaselineEnvironment::capture(MeasurementProtocol::Cold);

        let state_json = serde_json::to_string(&env.environment_state).unwrap();
        let state_back: EnvironmentState = serde_json::from_str(&state_json).unwrap();
        assert_eq!(state_back, env.environment_state);

        let a11y_json = serde_json::to_string(&env.accessibility).unwrap();
        let a11y_back: Accessibility = serde_json::from_str(&a11y_json).unwrap();
        assert_eq!(a11y_back, env.accessibility);

        let hw_json = serde_json::to_string(&env.reference_hardware).unwrap();
        let hw_back: ReferenceHardware = serde_json::from_str(&hw_json).unwrap();
        assert_eq!(hw_back, env.reference_hardware);

        let build_json = serde_json::to_string(&env.build_environment).unwrap();
        let build_back: BuildEnvironment = serde_json::from_str(&build_json).unwrap();
        assert_eq!(build_back, env.build_environment);
    }
}
