#!/usr/bin/env bash
# Fix inotify watch limit for large Rust/Node projects.
# Run with: sudo bash scripts/fix-inotify-limit.sh

set -euo pipefail

echo "==> Increasing inotify limits..."

# Apply immediately
sysctl -w fs.inotify.max_user_watches=524288
sysctl -w fs.inotify.max_user_instances=1024

# Persist across reboots
CONF="/etc/sysctl.d/99-kria-inotify.conf"
cat > "$CONF" <<'EOF'
# KRIA: increased inotify limits for cargo tauri dev / large projects
fs.inotify.max_user_watches=524288
fs.inotify.max_user_instances=1024
EOF

echo "==> Persisted to $CONF"
echo "==> Done. Limits active now and will survive reboots."
