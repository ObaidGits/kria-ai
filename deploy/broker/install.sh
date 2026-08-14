#!/usr/bin/env bash
#
# Install the KRIA privilege broker. This is the ONE step that needs root.
#
# Run:  sudo bash deploy/broker/install.sh
#
# What it does, in order:
#   1. builds the broker binary (as the invoking user, not as root)
#   2. installs it to /usr/local/lib/kria/
#   3. installs the Polkit policy, so each privileged action asks YOU for
#      confirmation through your desktop's own password dialog
#   4. installs and starts the hardened systemd unit
#
# What it deliberately does NOT do:
#   * enable package installation — that needs the opt-in drop-in, see
#     deploy/broker/10-packages.conf and read the warning in it first
#   * change any of your existing system configuration
#
# To undo everything:  sudo bash deploy/broker/install.sh --uninstall

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB_DIR="/usr/local/lib/kria"
BIN_NAME="kria-os-broker"
UNIT_NAME="kria-os-broker.service"
UNIT_DEST="/etc/systemd/system/${UNIT_NAME}"
POLICY_SRC="${REPO_ROOT}/crates/kria-core/src/os_control/broker/packaging/org.kria.broker.policy"
POLICY_DEST="/usr/share/polkit-1/actions/org.kria.broker.policy"

die() { printf 'error: %s\n' "$1" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "run with sudo: sudo bash deploy/broker/install.sh"

if [[ "${1:-}" == "--uninstall" ]]; then
    systemctl disable --now "${UNIT_NAME}" 2>/dev/null || true
    rm -f "${UNIT_DEST}" "${POLICY_DEST}" "${LIB_DIR}/${BIN_NAME}"
    rm -rf "/etc/systemd/system/${UNIT_NAME}.d"
    systemctl daemon-reload
    printf 'KRIA broker removed. Privileged actions will report "not available" again.\n'
    exit 0
fi

# The user who invoked sudo. Building as root would leave root-owned artefacts in
# the repository's target directory, which then breaks the next ordinary build.
BUILD_USER="${SUDO_USER:-}"
[[ -n "${BUILD_USER}" ]] || die "cannot determine the invoking user; run via sudo, not as root directly"

BUILT="${REPO_ROOT}/target/release/${BIN_NAME}"

if [[ -x "${BUILT}" ]]; then
    printf '1/4 reusing the already-built broker binary\n'
else
    # `sudo` resets PATH to secure_path, which does NOT include ~/.cargo/bin. So
    # cargo is resolved by absolute path from the invoking user's home rather than
    # by name — otherwise this step fails with "cargo: command not found" only on
    # machines where rustup installed cargo per-user, which is most of them.
    BUILD_HOME="$(getent passwd "${BUILD_USER}" | cut -d: -f6)"
    CARGO=""
    for candidate in "${BUILD_HOME}/.cargo/bin/cargo" /usr/local/bin/cargo /usr/bin/cargo; do
        [[ -x "${candidate}" ]] && { CARGO="${candidate}"; break; }
    done
    [[ -n "${CARGO}" ]] || die "cannot find cargo; build it first with: cargo build --release -p kria-core --no-default-features --features os-control-live --bin ${BIN_NAME}"

    printf '1/4 building the broker as %s (this can take several minutes) ...\n' "${BUILD_USER}"
    # -j 2 keeps peak memory low enough for a laptop.
    sudo -u "${BUILD_USER}" env --chdir="${REPO_ROOT}" HOME="${BUILD_HOME}" \
        "${CARGO}" build --release -p kria-core \
        --no-default-features --features os-control-live \
        --bin "${BIN_NAME}" -j 2
fi

[[ -f "${BUILT}" ]] || die "build did not produce ${BUILT}"

printf '2/4 installing the binary ...\n'
install -d -m 0755 "${LIB_DIR}"
# 0755 root-owned: writable only by root, so an unprivileged process cannot swap
# the binary that systemd will next start as root.
install -m 0755 -o root -g root "${BUILT}" "${LIB_DIR}/${BIN_NAME}"

printf '3/4 installing the Polkit policy ...\n'
[[ -f "${POLICY_SRC}" ]] || die "missing Polkit policy at ${POLICY_SRC}"
install -m 0644 -o root -g root "${POLICY_SRC}" "${POLICY_DEST}"

printf '4/4 installing and starting the service ...\n'
install -m 0644 -o root -g root "${REPO_ROOT}/deploy/broker/${UNIT_NAME}" "${UNIT_DEST}"
systemctl daemon-reload
systemctl enable --now "${UNIT_NAME}"

sleep 1
if systemctl is-active --quiet "${UNIT_NAME}"; then
    printf '\nDone. The broker is running.\n'
    printf 'Privileged actions (file ownership, firewall, printer setup, battery limits)\n'
    printf 'will now ask for your password through your desktop'"'"'s own dialog.\n'
else
    printf '\nThe service did not start. Diagnose with:\n'
    printf '  systemctl status %s\n  journalctl -u %s -n 50\n' "${UNIT_NAME}" "${UNIT_NAME}"
    exit 1
fi
