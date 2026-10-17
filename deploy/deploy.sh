#!/usr/bin/env bash
# Builds the cubby image and installs the Quadlet units for the invoking user, then restarts
# the server and enables the worker timer. Run it from the production checkout (~/cubby), never
# from a worktree.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
quadlet_dir="${XDG_CONFIG_HOME:-$HOME/.config}/containers/systemd"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

podman build -t localhost/cubby:latest -f "$root/Containerfile" "$root"

mkdir -p "$quadlet_dir" "$unit_dir" "$HOME/.config/cubby"
install -m 0644 "$root/deploy/cubby-server.container" "$quadlet_dir/"
install -m 0644 "$root/deploy/cubby-worker.service" "$root/deploy/cubby-worker.timer" "$unit_dir/"

systemctl --user daemon-reload
systemctl --user restart cubby-server.service
systemctl --user enable --now cubby-worker.timer
echo "==> cubby deployed; check: curl -fsS http://127.0.0.1:8790/healthz"
