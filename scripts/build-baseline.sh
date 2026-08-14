#!/usr/bin/env bash
#
# Measure what a build actually costs: wall time, peak memory, and the lowest the
# machine's free memory got.
#
#   bash scripts/build-baseline.sh <label> <command...>
#
# Results append to docs/build-baseline.md so Stage 7 can compare like for like.
#
# ── Why not /usr/bin/time -v ─────────────────────────────────────────────────
# Its "Maximum resident set size" comes from getrusage(RUSAGE_CHILDREN), which
# reports the largest SINGLE child — not the sum of concurrent ones. With `-j 2`
# there are two rustc processes plus cargo, so that number understates the real
# cost by roughly the parallelism factor. This samples the whole process tree and
# sums it, which is the number that decides whether the laptop swaps.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${REPO_ROOT}/docs/build-baseline.md"
LABEL="${1:?usage: build-baseline.sh <label> <command...>}"
shift

mkdir -p "$(dirname "${OUT}")"

SAMPLE_FILE="$(mktemp)"
STOP_FILE="$(mktemp)"
rm -f "${STOP_FILE}"

# Sampler: every 0.5s record (a) summed RSS of cargo/rustc/mold/ld in MB and
# (b) system available memory in MB.
sampler() {
    while [[ ! -f "${STOP_FILE}" ]]; do
        local rss avail
        rss=$(ps -eo rss=,comm= 2>/dev/null \
            | awk '$2 ~ /^(cargo|rustc|mold|ld|ld\.lld|collect2|cc1plus)$/ {s+=$1} END {print int(s/1024)}')
        avail=$(awk '/MemAvailable/ {print int($2/1024)}' /proc/meminfo)
        printf '%s %s\n' "${rss:-0}" "${avail:-0}" >> "${SAMPLE_FILE}"
        sleep 0.5
    done
}
sampler &
SAMPLER_PID=$!

START=$(date +%s)
"$@" > /tmp/build-baseline-output.txt 2>&1
STATUS=$?
END=$(date +%s)

touch "${STOP_FILE}"
wait "${SAMPLER_PID}" 2>/dev/null

PEAK_RSS=$(awk '{if ($1>m) m=$1} END {print m+0}' "${SAMPLE_FILE}")
MIN_AVAIL=$(awk 'NR==1{m=$2} {if ($2<m) m=$2} END {print m+0}' "${SAMPLE_FILE}")
SAMPLES=$(wc -l < "${SAMPLE_FILE}")
ELAPSED=$((END - START))

{
    printf '| %s | %ds | %s MB | %s MB | %s | %s |\n' \
        "${LABEL}" "${ELAPSED}" "${PEAK_RSS}" "${MIN_AVAIL}" "${SAMPLES}" \
        "$([[ ${STATUS} -eq 0 ]] && echo ok || echo "FAILED(${STATUS})")"
} >> "${OUT}"

printf '%-46s %4ds  peak %5s MB  min-free %5s MB  %s\n' \
    "${LABEL}" "${ELAPSED}" "${PEAK_RSS}" "${MIN_AVAIL}" \
    "$([[ ${STATUS} -eq 0 ]] && echo ok || echo "FAILED — see /tmp/build-baseline-output.txt")"

rm -f "${SAMPLE_FILE}" "${STOP_FILE}"
exit "${STATUS}"
