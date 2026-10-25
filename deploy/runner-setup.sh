#!/usr/bin/env bash
# Installs and registers the GitHub Actions runner that deploy.yml targets (label `cubby-prod`) on
# the production host, rootless, as a systemd user service of the invoking user (the same user that
# owns ~/cubby-prod and the Quadlet units). Same setup as dumpstar's prod runner.
#
# Registration needs a one-time token. As a repo admin, from any machine with gh:
#     gh api -X POST repos/DeckDumpster/cubby/actions/runners/registration-token --jq .token
# then on the host:
#     CUBBY_RUNNER_TOKEN=<token> deploy/runner-setup.sh
# Re-running with the runner already configured only refreshes the service.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prod_host="$(tr -d '[:space:]' < "$root/deploy/prod-host")"
this_host="${CUBBY_DEPLOY_HOSTNAME:-$(hostname)}"
if [ "$this_host" != "$prod_host" ]; then
    echo "refusing on '$this_host': the cubby-prod runner lives on '$prod_host' (deploy/prod-host)" >&2
    exit 1
fi
dir="${CUBBY_RUNNER_DIR:-$HOME/actions-runner-cubby}"
repo_url="${CUBBY_RUNNER_REPO_URL:-https://github.com/DeckDumpster/cubby}"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

if [ ! -x "$dir/config.sh" ]; then
    mkdir -p "$dir"
    ver="$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest | sed -n 's/.*"tag_name": *"v\([0-9.]*\)".*/\1/p')"
    [ -n "$ver" ] || { echo "runner-setup: could not resolve the latest runner release" >&2; exit 1; }
    curl -fsSL "https://github.com/actions/runner/releases/download/v$ver/actions-runner-linux-x64-$ver.tar.gz" | tar -xz -C "$dir"
fi
if [ ! -e "$dir/.runner" ]; then
    : "${CUBBY_RUNNER_TOKEN:?set CUBBY_RUNNER_TOKEN to a registration token (see the header of this script)}"
    "$dir/config.sh" --unattended --url "$repo_url" --token "$CUBBY_RUNNER_TOKEN" \
        --name "cubby-prod-$prod_host" --labels cubby-prod --replace
fi

mkdir -p "$unit_dir"
cat > "$unit_dir/cubby-runner.service" <<UNIT
[Unit]
Description=GitHub Actions runner (cubby-prod)
After=network-online.target

[Service]
ExecStart=$dir/run.sh
WorkingDirectory=$dir
Restart=always
RestartSec=10
KillMode=process

[Install]
WantedBy=default.target
UNIT
loginctl enable-linger "$USER"
systemctl --user daemon-reload
systemctl --user enable --now cubby-runner.service
systemctl --user is-active cubby-runner.service
