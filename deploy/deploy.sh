#!/usr/bin/env bash
# Builds the cubby image and installs the Quadlet units for the invoking user, then restarts
# the server and enables the worker timer. Run it from the production checkout (~/cubby-prod on the
# host named in deploy/prod-host), never from a worktree. It refuses on any other host: the Spira box
# (agent-swarm-test) runs nothing but Spira.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prod_host="$(tr -d '[:space:]' < "$root/deploy/prod-host")"
this_host="${CUBBY_DEPLOY_HOSTNAME:-$(hostname)}"
if [ "$this_host" != "$prod_host" ]; then
    echo "refusing to deploy on '$this_host': Cubby production is '$prod_host' (deploy/prod-host)" >&2
    exit 1
fi
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
# Without the Access settings the server answers "Cloudflare Access is not configured" to every
# parent request. Fill them from Terraform if absent, and refuse to (re)start the server without them.
access_missing() {
    local k out=()
    for k in CUBBY_ACCESS_TEAM_DOMAIN CUBBY_ACCESS_AUD CUBBY_ACCESS_JWKS_URL; do
        grep -qE "^$k=.+" "$cfg_dir/server.env" || out+=("$k")
    done
    echo "${out[*]:-}"
}
if [ -n "$(access_missing)" ]; then
    "$root/deploy/access-env.sh" || echo "==> deploy/access-env.sh failed" >&2
    missing="$(access_missing)"
    if [ -n "$missing" ]; then
        echo "==> $cfg_dir/server.env lacks: $missing" >&2
        echo "    /parent would reject everyone; run deploy/access-env.sh (Terraform outputs) and redeploy" >&2
        exit 1
    fi
fi
# The worker's keys (CUBBY_YT_API_KEY, CUBBY_OTLP_ENDPOINT) are all optional, but podman --env-file
# exits 125 on a missing file, so the timer would fail every firing. Create it empty.
[ -e "$cfg_dir/worker.env" ] || install -m 0600 /dev/null "$cfg_dir/worker.env"
# The server ingests a link's metadata when a parent allows it, so it needs the worker's API key too.
if ! grep -q '^CUBBY_YT_API_KEY=' "$cfg_dir/server.env" && grep -q '^CUBBY_YT_API_KEY=' "$cfg_dir/worker.env"; then
    grep '^CUBBY_YT_API_KEY=' "$cfg_dir/worker.env" >> "$cfg_dir/server.env"
fi

# Metrics: the OTLP receiver (VictoriaMetrics) on this host, named in deploy/otlp-endpoint and
# reached from the containers through the host alias. Set in both env files on every deploy.
otlp="$(tr -d '[:space:]' < "$root/deploy/otlp-endpoint")"
set_env() {
    local f="$cfg_dir/$1" tmp
    tmp="$(mktemp "$f.XXXXXX")"
    { grep -v '^CUBBY_OTLP_ENDPOINT=' "$f" || true; echo "CUBBY_OTLP_ENDPOINT=$otlp"; } >"$tmp"
    chmod 0600 "$tmp"; mv "$tmp" "$f"
}
set_env server.env
set_env worker.env

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
# A deploy is done when a user can walk the product, not when /healthz answers: five breaks reached
# the operator one at a time behind a green healthz. The walk's output is part of this log.
if ! run "$root/deploy/smoke.sh"; then
    echo "==> deploy FAILED: the smoke walk (deploy/smoke.sh) failed; the instance is up but a user path is broken" >&2
    exit 1
fi
echo "==> cubby deployed and walked; check: curl -fsS http://127.0.0.1:8790/healthz"
