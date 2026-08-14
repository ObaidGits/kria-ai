# Build cost baseline — before and after splitting `kria-core`

Measured by `scripts/build-baseline.sh`, which samples the whole `cargo` + `rustc`
process tree every 0.5s. **Peak MB** is the summed resident memory of that tree, not
the largest single process — with `-j 2` there are two compilers plus cargo, so a
single-process figure would understate the real cost by roughly the parallelism.

**Min free MB** is the lowest `MemAvailable` reached during the build. That is the
number that decides whether the laptop starts swapping.

## Why these three measurements

| Measurement | What it proves |
|---|---|
| Full build from clean | The worst case, and the ceiling on memory |
| Rebuild after one central edit | **The number the split is meant to fix** — today it recompiles the whole crate because incremental is off and everything is one compilation unit |
| Rebuild after one leaf edit | Control: shows the floor a split could approach |

## Results

| Scenario | Wall | Peak MB | Min free MB | Samples | Status |
|---|---|---|---|---|---|| harness self-test (cargo metadata) | 0s | 0 MB | 7091 MB | 1 | ok |
| BEFORE · rebuild after CENTRAL edit (os_control/context.rs) | 267s | 8090 MB | 454 MB | 509 | ok |
| BEFORE · rebuild after LEAF edit (mod.rs) | 267s | 7981 MB | 385 MB | 511 | ok |

## What the BEFORE numbers prove

**A leaf edit costs exactly as much as a central edit: 267s both.**

That is the whole case for splitting the crate, and it is now measured rather than
asserted. `notify/mod.rs` is 254 lines that nothing else in the crate depends on;
`os_control/context.rs` is a type every governed action flows through. Touching
either one costs the same 4½ minutes and ~8 GB, because Cargo's unit of
recompilation is the **crate**, not the file — and `kria-core` is one crate holding
~517,000 lines.

Peak memory of ~8 GB on a 15 GB machine also explains the stalls: free memory fell
to **385–454 MB** and the machine leaned on swap (5.6 GB in use at the trough).

A successful split changes the leaf-edit row, not the central-edit one. After it,
editing `notify` should only rebuild the crate that contains `notify` — so the
useful comparison in Stage 7 is **leaf edit before vs leaf edit after**.

## Deliberately not measured: a full build from clean

`cargo clean` would have thrown away the whole `target/` directory, making every
later stage of this refactor slower for no extra insight — a from-clean build has to
compile everything once whether the code is one crate or six, so it is the one
scenario a split barely improves. The two rows above are the ones that move.
| AFTER Stage 2 · rebuild after LEAF edit (notify/mod.rs) | 246s | 8630 MB | 861 MB | 474 | ok |
| AFTER Stage 2 · LEAF edit, LIBRARY ONLY (cargo check) | 46s | 4233 MB | 5611 MB | 89 | ok |

## Stage 2 result — and why it changes the plan

`memory` (105,006 lines, 20% of the crate) was extracted successfully: 1,960 of its
tests moved with it, kria-core kept the other 3,737, and the total is unchanged at
5,697. Nothing was lost and nothing broke.

But the payoff was **8%**, not the step change the plan assumed:

| Scenario | Wall | Peak MB | Min free MB |
|---|---|---|---|
| BEFORE · leaf edit, full test build | 267s | 7,981 | 385 |
| AFTER Stage 2 · leaf edit, full test build | 246s | 8,630 | 861 |
| AFTER Stage 2 · leaf edit, **library only** | **46s** | **4,233** | 5,611 |

The third row is the one that matters. Compiling the library after a leaf edit costs
**46 seconds**. The other **200 seconds — 80% of the wall time — is building and
linking 152 separate integration-test binaries**, each of which links the entire
crate.

Splitting the crate reduces the 46s part in proportion to what is removed. It does
**nothing** for the 200s part. That is why removing a fifth of the code bought only
8%: the thing being optimised was never the bottleneck.

### What actually attacks the 200 seconds

1. **The `mold` linker** — not yet installed. Linking is the dominant cost and mold
   exists for precisely this. `sudo bash scripts/enable-mold.sh`.
2. **Fewer test binaries** — 152 files in `crates/kria-core/tests/` become 152
   binaries, each statically linking the whole crate. Consolidating them into a
   handful of binaries with modules inside would cut most of that linking. The tests
   themselves would not change, only how many executables they are packed into.

Stages 3–6 (voice, image, resource, os_control) were planned on the assumption that
crate size drove the cost. The measurement says otherwise, so they are **not worth
doing for build speed**. They may still be worth doing later for architectural
clarity — that is a different justification, and should be argued on its own terms.
| AFTER consolidation · leaf edit, full test build (49 binaries) | 163s | 8829 MB | 825 MB | 315 | ok |
| AFTER mold + consolidation · leaf edit, full test build | 179s | 8498 MB | 970 MB | 342 | ok |
| AFTER mold (actually engaged) + consolidation · leaf edit | 303s | 8269 MB | 643 MB | 583 | ok |
| AFTER mold + consolidation · leaf edit, warm cache | 172s | 8521 MB | 588 MB | 329 | ok |

## Test-binary consolidation — the change that worked

152 integration-test files each became their own executable, statically linking the
whole crate. 109 of them make no process-global mutation and were grouped into 6
umbrella binaries; the other 43 keep their own process because they call
`env::set_var`, bind a port, or hold a global, and sharing would change how they
behave. 49 binaries instead of 152.

| Scenario | Wall | Peak MB |
|---|---|---|
| Stage 2 only · leaf edit | 246s | 8,630 |
| **+ consolidation** | **163s** | 8,829 |
| + consolidation + mold | 172s | 8,521 |

**163s, down from the original 267s — a 39% reduction.** All 1,912 test functions are
still present; nothing was dropped, only repacked.

### mold made no difference, and is left off

Linking was the hypothesis; the measurement refuted it. 163s without mold, 172s with
— no gain, within noise. Once the duplicated linking was gone, what remained was
rustc codegen, which a linker cannot help. mold stays installed but unwired; see
`.cargo/config.toml` for how to re-enable and why `-B` is the wrong flag.

The lesson repeats Stage 2's: measure the phases before optimising one. Both the
crate split and the linker were plausible fixes aimed at costs that were not actually
dominant.
