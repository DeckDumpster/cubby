#!/usr/bin/env bash
# Offline checks of the cutover artefacts: Litestream config,
# unit files and deploy idempotence. No network, no podman. Runs from deploy/ci.sh.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
fail() { echo "FAIL: $*" >&2; exit 1; }

# Access policy split and tunnel ingress are Terraform now: infra/tests/guarantees.tftest.hcl.

# --- Litestream: directory mode over the data volume, bucket cubby-backups, top-level retention.
grep -q 'dir: /data' litestream.yml || fail "litestream dir"
grep -q 'pattern: "\*.sqlite"' litestream.yml || fail "litestream pattern"
grep -q 'bucket: cubby-backups' litestream.yml || fail "litestream bucket"
grep -q '^snapshot:' litestream.yml || fail "snapshot block must be top level"
grep -q '^Image=docker.io/litestream/litestream' cubby-litestream.container || fail "litestream image"
grep -q 'Volume=cubby-data:/data' cubby-litestream.container || fail "litestream volume"

# --- Restore drill refuses without a replica URL and without a target.
if ./restore-drill.sh >/dev/null 2>&1; then fail "restore-drill ran with no arguments"; fi
h="$(./restore-drill.sh --help)"; case "$h" in *family-*) ;; *) fail "restore-drill help";; esac; : || fail "restore-drill help"

# --- Tunnel connector: host network to reach loopback, token from an env file, never inline.
grep -q '^Network=host' cubby-cloudflared.container || fail "cloudflared must use host network"
grep -q '^EnvironmentFile=.*cloudflared.env' cubby-cloudflared.container || fail "cloudflared env file"
if grep -qi 'TUNNEL_TOKEN=.\+\|--token' cubby-cloudflared.container; then fail "token inline in unit"; fi

# --- Quadlet accepts no optional "-" prefix on EnvironmentFile=: it turns it into a relative path
# and podman exits 125. Static check always; render with the real generator when it is installed.
if grep -n '^EnvironmentFile=-' *.container; then fail "EnvironmentFile= with '-' prefix in a .container"; fi
quadlet=/usr/libexec/podman/quadlet
if [ -x "$quadlet" ]; then
    q="$(mktemp -d)"; cp *.container "$q/"
    out="$(QUADLET_UNIT_DIRS="$q" "$quadlet" -user -dryrun 2>&1)" || fail "quadlet generator failed: $out"
    printf '%s\n' "$out" | grep -q -- '--env-file' || fail "quadlet rendered no --env-file"
    printf '%s\n' "$out" | grep -o -- '--env-file [^ ]*' | grep -Ev -- '--env-file (/|%h/)' && fail "unresolvable (relative) env-file path"
    rm -rf "$q"
else
    echo "quadlet generator not installed: static check only"
fi

# --- Deploy is idempotent: running the install step twice leaves identical files.
t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
for i in 1 2; do CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null; done
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum1"
CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum2"
cmp "$t.sum1" "$t.sum2" || fail "deploy not idempotent"
[ -f "$t/containers/systemd/cubby-litestream.container" ] || fail "litestream unit not installed"
[ -f "$t/containers/systemd/cubby-cloudflared.container" ] || fail "cloudflared unit not installed"
rm -f "$t.sum1" "$t.sum2"
[ -f "$t/cubby/server.env" ] || fail "server.env not created"
echo "deploy artefacts ok"
