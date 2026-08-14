#!/usr/bin/env bash
#
# Measure the MARGINAL cost of one integration-test binary.
#
# Why this measurement decides the plan
# -------------------------------------
# A leaf edit costs 246s: ~46s to compile the library, ~200s for the 152 test
# binaries. But that 200s is two different things added together:
#
#   (a) COMPILING 73,237 lines of test code   — consolidation does NOT remove this
#   (b) LINKING 152 separate executables      — consolidation DOES remove most of it
#
# Only (b) is recoverable. If the 200s is mostly (a), consolidating buys little and
# is not worth touching 152 files for. So measure the slope: build N test targets,
# then N+10, and attribute the difference to the extra binaries.
#
# Usage: bash scripts/measure-test-binary-cost.sh
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

FEATURES=(--no-default-features --features os-control-test)
mapfile -t ALL < <(ls crates/kria-core/tests/*.rs | xargs -n1 basename | sed 's/\.rs$//' | sort)
echo "total test targets available: ${#ALL[@]}"

build() {                       # build() <label> <target...>
  local label="$1"; shift
  local args=()
  for t in "$@"; do args+=(--test "$t"); done
  # Touch the library so every run redoes the same lib work, making the runs
  # comparable — otherwise the second run reuses the first's cached lib.
  touch crates/kria-core/src/notify/mod.rs
  local start; start=$(date +%s)
  cargo test -p kria-core "${FEATURES[@]}" "${args[@]}" --no-run -j 2 >/tmp/mtbc.log 2>&1
  local rc=$?; local end; end=$(date +%s)
  printf '%-28s %3ds  (rc=%d, %d targets)\n' "$label" "$((end - start))" "$rc" "$#"
  [ $rc -ne 0 ] && tail -5 /tmp/mtbc.log
  echo "$((end - start))"
}

# 1 target: lib cost + one binary.
T1=$(build "1 test binary" "${ALL[0]}" | tail -1)
# 11 targets: same lib cost + eleven binaries.
T11=$(build "11 test binaries" "${ALL[@]:0:11}" | tail -1)

echo
echo "lib + 1 binary   : ${T1}s"
echo "lib + 11 binaries: ${T11}s"
if [ "$T11" -gt "$T1" ]; then
  echo "marginal cost per extra binary: $(( (T11 - T1) / 10 ))s"
  echo "projected for all 152          : $(( T1 + (T11 - T1) / 10 * 151 ))s"
else
  echo "no measurable slope — linking is not the dominant cost"
fi
