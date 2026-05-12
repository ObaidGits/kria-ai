# KRIA developer task runner.
#
# All `test-*` targets respect the safety rules in
# docs/GUI_INTELLIGENCE_REVIEW.md Appendix D: no real DISPLAY, no real HOME,
# no real sudo. Targets that boot a virtual display use Xvfb at :99.

# Show available recipes
default:
    @just --list

# ── Fast unit / property / integration tests ────────────────────────────────
# Runs everything that does NOT need a virtual display. Default `cargo test`.
test:
    cargo test --workspace --lib

# Same but with the cognition-v2 feature flag enabled.
test-cognition:
    cargo test -p kria-core --features gui_cognition_v2 --lib

# ── Daemon protocol integration tests ──────────────────────────────────────
# Spawns the real kria-uinput-daemon on a disposable socket. Does NOT touch
# the user's display.
test-daemon:
    cargo test -p kria-uinput-daemon --features daemon-it

# ── Safe E2E GUI tests under Xvfb (no real desktop touched) ────────────────
# Boots Xvfb at :99 with a throw-away HOME, runs the stub app under that
# display, then runs the e2e-xvfb-tagged tests serially.
test-e2e:
    @echo "Booting sandboxed Xvfb session for safe GUI E2E…"
    @mkdir -p /tmp/kria-test-home
    @# Refuse to run if user's real session is :0/:1
    @if [ "${DISPLAY:-}" = ":0" ] || [ "${DISPLAY:-}" = ":1" ]; then \
        echo "REFUSING: real DISPLAY $DISPLAY detected. Run from a TTY."; exit 1; \
     fi
    @if ! command -v Xvfb >/dev/null; then \
        echo "Xvfb not installed. apt install xvfb"; exit 1; \
     fi
    @Xvfb :99 -screen 0 1920x1200x24 &
    @sleep 1
    DISPLAY=:99 \
    HOME=/tmp/kria-test-home \
    XDG_CONFIG_HOME=/tmp/kria-test-home/.config \
    XDG_DATA_HOME=/tmp/kria-test-home/.local/share \
    KRIA_TEST_DUMP_PATH=/tmp/kria-test-home/dump.txt \
    cargo test --workspace --features e2e-xvfb -- --ignored --test-threads=1 || \
        ( pkill -f "Xvfb :99" ; exit 1 )
    @pkill -f "Xvfb :99" || true

# ── Adversarial E2E tests ──────────────────────────────────────────────────
# Same sandbox; runs the destructive/deceptive scenarios from §D.4.
test-adversarial:
    @echo "Running adversarial GUI test scenarios (Xvfb sandbox)…"
    DISPLAY=:99 \
    HOME=/tmp/kria-test-home \
    KRIA_TEST_DUMP_PATH=/tmp/kria-test-home/dump.txt \
    cargo test --workspace --features e2e-xvfb,adversarial -- --ignored

# ── Build everything (release) ─────────────────────────────────────────────
build-release:
    cargo build --release --workspace

# Build just the uinput daemon (release + debug).
build-daemon:
    cargo build --release -p kria-uinput-daemon
    cargo build -p kria-uinput-daemon

# ── Lint / format ──────────────────────────────────────────────────────────
fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-features -- -D warnings
