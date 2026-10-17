#!/usr/bin/env bash
# Builds the cubby image and installs the Quadlet units for the invoking user, then restarts
# the server and enables the worker timer. Run it from the production checkout (~/cubby), never
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
install -m 0644 "$root/deploy/cubby-server.container" "$root/deploy/cubby-litestream.container" "$quadlet_dir/"
install -m 0644 "$root/deploy/litestream.yml" "$HOME_CFG/litestream.yml"
install -m 0644 "$root/deploy/cubby-worker.service" "$root/deploy/cubby-worker.timer" "$unit_dir/"

run systemctl --user daemon-reload
run systemctl --user restart cubby-server.service
run systemctl --user enable --now cubby-worker.timer
# The replica needs its env file (S3 credentials); without it, skip rather than error-loop.
if [ -f "$cfg_dir/litestream.env" ]; then run systemctl --user restart cubby-litestream.service
else echo "==> $cfg_dir/litestream.env missing: litestream not started (needs the S3 grant)"; fi
echo "==> cubby deployed; check: curl -fsS http://127.0.0.1:8790/healthz"
