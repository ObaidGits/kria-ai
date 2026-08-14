#!/usr/bin/env bash
#
# Install the `mold` linker and point Rust at it.
#
# Run:  sudo bash scripts/enable-mold.sh
# Undo: sudo bash scripts/enable-mold.sh --disable
#
# ── Why ──────────────────────────────────────────────────────────────────────
# Linking is the memory peak of a KRIA test build. The default GNU `ld` holds the
# whole link graph of a 517k-line crate in memory at once; `mold` is designed for
# exactly this case and typically cuts both link time and peak memory several-fold.
#
# ── Why this is safe ─────────────────────────────────────────────────────────
# A linker only decides HOW the pieces are joined, not what the code does. The
# resulting binary behaves identically. Nothing about the app changes.
#
# ── Why one script instead of "install it, then edit the config" ──────────────
# A cargo config naming a linker that is not installed breaks EVERY build with a
# confusing "linker not found". So the config is written only after the install
# succeeds, and removed again on --disable. The two steps are never out of sync.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="${REPO_ROOT}/.cargo/config.toml"
MARKER_BEGIN="# ── mold linker (managed by scripts/enable-mold.sh) ───────────"
MARKER_END="# ── end mold linker ──────────────────────────────────────────"

die() { printf 'error: %s\n' "$1" >&2; exit 1; }

remove_block() {
    # Strip a previously managed block, if present.
    if grep -qF "${MARKER_BEGIN}" "${CONFIG}"; then
        python3 - "$CONFIG" "$MARKER_BEGIN" "$MARKER_END" <<'PY'
import sys
path, begin, end = sys.argv[1], sys.argv[2], sys.argv[3]
lines = open(path, encoding="utf-8").read().split("\n")
out, skipping = [], False
for line in lines:
    if line.startswith(begin[:20]):
        skipping = True
        continue
    if skipping and line.startswith(end[:20]):
        skipping = False
        continue
    if not skipping:
        out.append(line)
open(path, "w", encoding="utf-8").write("\n".join(out))
PY
    fi
}

if [[ "${1:-}" == "--disable" ]]; then
    remove_block
    printf 'mold disabled. Rust is back on the default linker.\n'
    printf 'The package itself is left installed; remove it with: sudo apt remove mold\n'
    exit 0
fi

[[ $EUID -eq 0 ]] || die "run with sudo: sudo bash scripts/enable-mold.sh"

printf '1/3 installing mold ...\n'
if command -v mold >/dev/null 2>&1; then
    printf '      already installed (%s)\n' "$(command -v mold)"
else
    apt-get update -qq
    apt-get install -y mold
fi

MOLD="$(command -v mold)" || die "mold is still not on PATH after install"
printf '      using %s\n' "${MOLD}"

printf '2/3 wiring cargo to use it ...\n'
remove_block
cat >> "${CONFIG}" <<EOF

${MARKER_BEGIN}
# Linking is the memory peak of a test build; mold is far lighter than GNU ld.
# A linker changes only HOW objects are joined — the binary behaves identically.
# Managed block: re-run scripts/enable-mold.sh --disable to remove it.
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=${MOLD}"]
${MARKER_END}
EOF

# The config belongs to the invoking user, not root — otherwise the next ordinary
# cargo run cannot rewrite it.
if [[ -n "${SUDO_USER:-}" ]]; then
    chown "${SUDO_USER}" "${CONFIG}" 2>/dev/null || true
fi

printf '3/3 verifying the config parses ...\n'
BUILD_USER="${SUDO_USER:-root}"
BUILD_HOME="$(getent passwd "${BUILD_USER}" | cut -d: -f6)"
CARGO="${BUILD_HOME}/.cargo/bin/cargo"
[[ -x "${CARGO}" ]] || CARGO="$(command -v cargo || true)"
if [[ -x "${CARGO}" ]]; then
    # `metadata` parses the manifests and config WITHOUT compiling anything, so
    # this is a seconds-long check rather than a build.
    sudo -u "${BUILD_USER}" env --chdir="${REPO_ROOT}" HOME="${BUILD_HOME}" \
        "${CARGO}" metadata --no-deps --format-version 1 >/dev/null \
        || die "cargo rejected the config; the mold block was written but is invalid"
    printf '      config parses cleanly\n'
else
    printf '      cargo not found; skipped the parse check\n'
fi

printf '\nDone. The next build links with mold.\n'
printf 'First build after this is still a full rebuild (the linker changed).\n'
