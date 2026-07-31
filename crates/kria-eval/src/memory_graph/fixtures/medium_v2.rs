//! `mg-medium-v2` deterministic fixture generator (task F0.2 / 0.2.2).
//!
//! Seed `0x4D475203`. Produces exactly **10,000 records** for faults/rebuild
//! testing (`validation.md` §2). It plants every task-0.2.2 case via the shared
//! [`super::scene_v2`] builder at fault/rebuild scale: seven-destination states,
//! long/RTL/CJK labels, a large **outbox backlog**, multiple **model
//! partitions**, **corruption sentinels**, **interchange import** extensions,
//! and **source-cancellation** cases.
//!
//! All content is synthetic (no real private data); expected answers are the
//! independent oracle defined by the generator, never derived from a system
//! under test.

use super::scene_v2::{self, SceneParams};
use super::{FixtureGenerator, FixturePackage};

/// The frozen seed for `mg-medium-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5203;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-medium-v2";

/// Total records planted (`validation.md`: "10,000 records; faults/rebuild").
pub const TOTAL_RECORDS: usize = 10_000;

/// Parameters for the `mg-medium-v2` package (fault/rebuild scale).
pub const PARAMS: SceneParams = SceneParams {
    fixture_id: FIXTURE_ID,
    seed: SEED,
    generator_name: "memory_graph::fixtures::medium_v2",
    total_records: TOTAL_RECORDS,
    invalid_records: 24,
    // A real backlog plus more partitions/sentinels/imports/cancellations.
    outbox_items: 128,
    model_partitions: 4,
    corruption_sentinels: 40,
    import_candidates: 60,
    source_cancellations: 24,
};

/// The `mg-medium-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct MediumV2Generator;

impl FixtureGenerator for MediumV2Generator {
    fn fixture_id(&self) -> &'static str {
        FIXTURE_ID
    }

    fn seed(&self) -> u64 {
        SEED
    }

    fn generate(&self) -> FixturePackage {
        scene_v2::build(&PARAMS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5203);
        assert_eq!(FIXTURE_ID, "mg-medium-v2");
        assert_eq!(TOTAL_RECORDS, 10_000);
        let m = MediumV2Generator.generate().manifest;
        assert_eq!(m.generator.seed_hex, "0x4D475203");
    }

    #[test]
    fn distinct_from_small_seed() {
        // The two fixtures must not collide on seed or identity.
        assert_ne!(SEED, super::super::small_v2::SEED);
        assert_ne!(
            MediumV2Generator.generate().manifest.package_sha256,
            super::super::small_v2::SmallV2Generator
                .generate()
                .manifest
                .package_sha256
        );
    }

    #[test]
    fn scene_contract_holds() {
        scene_v2::test_support::assert_scene_contract(&PARAMS);
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let pkg = MediumV2Generator.generate();
        let dir = pkg.materialize(&root).expect("materialize package");
        for f in [
            "records.json",
            "links.json",
            "outbox.json",
            "partitions.json",
            "corruption.json",
            "imports.json",
            "sources.json",
            "fixture-manifest.json",
        ] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, MediumV2Generator.generate().manifest_bytes());
    }
}
