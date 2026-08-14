#!/usr/bin/env bash
# Run every os_control integration suite under the mandated deny-live invocation.
# Never runs live_smoke and never enables os-control-live: nothing touches the host.
set -u
cd /media/obaid/SSD/KRIA || exit 1

total_pass=0
total_fail=0
failed_suites=""

for f in crates/kria-core/tests/os_control_*.rs; do
    t=$(basename "$f" .rs)
    out=$(timeout 900 cargo test -p kria-core --no-default-features \
        --features os-control-test --test "$t" -j 2 2>&1)
    if echo "$out" | grep -qE "^error"; then
        printf "%-46s COMPILE ERRORS\n" "$t"
        failed_suites="$failed_suites $t"
        continue
    fi
    line=$(echo "$out" | grep -E "^test result" | head -1)
    p=$(echo "$line" | sed -n 's/.*ok\. \([0-9]*\) passed.*/\1/p')
    fl=$(echo "$line" | sed -n 's/.*\([0-9]*\) failed.*/\1/p')
    [ -z "$p" ] && p=0
    [ -z "$fl" ] && fl=0
    total_pass=$((total_pass + p))
    total_fail=$((total_fail + fl))
    if [ "$fl" != "0" ]; then
        printf "%-46s %s FAILED\n" "$t" "$fl"
        failed_suites="$failed_suites $t"
    else
        printf "%-46s %3s passed\n" "$t" "$p"
    fi
done

echo "───────────────────────────────────────────────────────"
echo "integration total: $total_pass passed, $total_fail failed"
[ -n "$failed_suites" ] && echo "failing:$failed_suites"
exit 0
