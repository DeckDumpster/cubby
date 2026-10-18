#!/usr/bin/env bash
# Builds the cubby image and installs the Quadlet units for the invoking user, then restarts
# the server and enables the worker timer. Run it from the production checkout (~/cubby-prod), never
# from a worktree.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cfg_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cubby"
HOME_CFG="$cfg_dir"
quadlet_dir="${XDG_CONFIG_HOME:-$HOME/.config}/containers/systemd"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

run() { if [ -n "${CUBBY_DEPLOY_DRY_RUN:-}" ]; then echo "dry-run: $*"; else "$@"; fi; }

run podman build -t localhost/cubby:latest -f "$root/Containerfile" "$root"

mkdir -p "$quadlet_dir" "$unit_dir" "$cfg_dir"
install -m 0644 "$root/deploy/cubby-server.container" "$root/deploy/cubby-litestream.container" "$root/deploy/cubby-cloudflared.container" "$quadlet_dir/"
install -m 0644 "$root/deploy/litestream.yml" "$HOME_CFG/litestream.yml"
install -m 0644 "$root/deploy/cubby-worker.service" "$root/deploy/cubby-worker.timer" "$unit_dir/"

# The server's env file is optional in content but must exist: Quadlet rejects a "-" prefix.
[ -e "$cfg_dir/server.env" ] || install -m 0600 /dev/null "$cfg_dir/server.env"

# Restart a unit; on failure print podman's own error (not a pointer to journalctl) and exit non-zero.
restart() {
    if run systemctl --user restart "$1"; then return 0; fi
    echo "==> $1 failed to start:" >&2
    journalctl --user -u "$1" -n 20 --no-pager -o cat >&2 || true
    exit 1
}

run systemctl --user daemon-reload
restart cubby-server.service
run systemctl --user enable --now cubby-worker.timer
# The replica needs its env file (S3 credentials); without it, skip rather than error-loop.
if [ -f "$cfg_dir/litestream.env" ]; then restart cubby-litestream.service
else echo "==> $cfg_dir/litestream.env missing: litestream not started (needs the S3 grant)"; fi
# The tunnel needs its token (deploy/tunnel-env.sh); without it, skip rather than error-loop.
if [ -f "$cfg_dir/cloudflared.env" ]; then restart cubby-cloudflared.service
else echo "==> $cfg_dir/cloudflared.env missing: tunnel not started (run deploy/tunnel-env.sh)"; fi
echo "==> cubby deployed; check: curl -fsS http://127.0.0.1:8790/healthz"
