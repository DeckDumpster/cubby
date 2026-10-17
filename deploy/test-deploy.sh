#!/usr/bin/env bash
# Offline checks of the cutover artefacts: Access policy split, tunnel route, Litestream config,
# unit files and deploy idempotence. No network, no podman. Runs from deploy/ci.sh.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
fail() { echo "FAIL: $*" >&2; exit 1; }

# --- Access: parent is gated, child and static are bypass, one app per path set.
acc=cloudflare/access.json
jq -e '.hostname == "cubby.gantt.pub"' "$acc" >/dev/null || fail "hostname"
jq -e '[.apps[] | select(.name=="cubby-parent") | .paths[]] | sort == ["/api/parent*","/parent*"]' "$acc" >/dev/null || fail "parent paths"
jq -e '.apps[] | select(.name=="cubby-parent") | .policy.decision == "allow" and (.policy.emails|length)==1' "$acc" >/dev/null || fail "parent policy must allow exactly one email"
jq -e '[.apps[] | select(.name=="cubby-public") | .paths[]] | index("/watch*") != null and index("/api/child*") != null' "$acc" >/dev/null || fail "child paths bypass"
jq -e '.apps[] | select(.name=="cubby-public") | .policy.decision == "bypass"' "$acc" >/dev/null || fail "public bypass"
jq -e '[.apps[].paths[]] | all(. != "/*" and . != "/")' "$acc" >/dev/null || fail "no catch-all app"

# --- Tunnel: ingress to loopback 8790 with a 404 catch-all.
grep -q 'hostname: cubby.gantt.pub' cloudflare/tunnel.yml || fail "tunnel hostname"
grep -q 'service: http://127.0.0.1:8790' cloudflare/tunnel.yml || fail "tunnel service"
tail -n1 cloudflare/tunnel.yml | grep -q 'http_status:404' || fail "tunnel catch-all"

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

# --- Deploy is idempotent: running the install step twice leaves identical files.
t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
for i in 1 2; do CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null; done
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum1"
CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum2"
cmp "$t.sum1" "$t.sum2" || fail "deploy not idempotent"
[ -f "$t/containers/systemd/cubby-litestream.container" ] || fail "litestream unit not installed"
rm -f "$t.sum1" "$t.sum2"
echo "deploy artefacts ok"
