//! `mg-small-v2` deterministic fixture generator (task F0.2 / 0.2.2).
//!
//! Seed `0x4D475202`. Produces exactly **1,000 records** for API/UI testing
//! (`validation.md` §2). It plants every task-0.2.2 case via the shared
//! [`super::scene_v2`] builder: seven-destination states, long/RTL/CJK labels,
//! empty/partial/stale/offline/recovery UI states, retrieval traces and
//! corrections, plus outbox/model/corruption/import/source-cancel cases.
//!
//! All content is synthetic (no real private data); expected answers are the
//! independent oracle defined by the generator, never derived from a system
//! under test.

use super::scene_v2::{self, SceneParams};
use super::{FixtureGenerator, FixturePackage};

/// The frozen seed for `mg-small-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5202;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-small-v2";

/// Total records planted (`validation.md`: "1,000 records; API/UI").
pub const TOTAL_RECORDS: usize = 1_000;

/// Parameters for the `mg-small-v2` package.
pub const PARAMS: SceneParams = SceneParams {
    fixture_id: FIXTURE_ID,
    seed: SEED,
    generator_name: "memory_graph::fixtures::small_v2",
    total_records: TOTAL_RECORDS,
    invalid_records: 8,
    outbox_items: 16,
    model_partitions: 2,
    corruption_sentinels: 4,
    import_candidates: 6,
    source_cancellations: 4,
};

/// The `mg-small-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct SmallV2Generator;

impl FixtureGenerator for SmallV2Generator {
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
        assert_eq!(SEED, 0x4D47_5202);
        assert_eq!(FIXTURE_ID, "mg-small-v2");
        assert_eq!(TOTAL_RECORDS, 1_000);
        let m = SmallV2Generator.generate().manifest;
        assert_eq!(m.generator.seed_hex, "0x4D475202");
    }

    #[test]
    fn scene_contract_holds() {
        scene_v2::test_support::assert_scene_contract(&PARAMS);
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let pkg = SmallV2Generator.generate();
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
        // Re-materialization is byte-stable.
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, SmallV2Generator.generate().manifest_bytes());
    }
}
