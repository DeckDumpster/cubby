#!/usr/bin/env bash
# Offline checks of the cutover artefacts: Access policy split, tunnel route, Litestream config,
# unit files and deploy idempotence. No network, no podman. Runs from deploy/ci.sh.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
fail() { echo "FAIL: $*" >&2; exit 1; }

# --- Access: parent is gated, child and static are bypass, one app per path set.
acc=cloudflare/access.json
# node, not jq: ci.sh guarantees node on PATH, the testenv image has no jq.
node - "$acc" <<'JS' || fail "access policy split"
const a = JSON.parse(require("fs").readFileSync(process.argv[2], "utf8"));
const bad = (m) => { console.error("access: " + m); process.exit(1); };
if (a.hostname !== "cubby.gantt.pub") bad("hostname");
const app = (n) => a.apps.find((x) => x.name === n) || bad("missing app " + n);
const parent = app("cubby-parent"), pub = app("cubby-public");
if (JSON.stringify([...parent.paths].sort()) !== JSON.stringify(["/api/parent*", "/parent*"])) bad("parent paths");
if (parent.policy.decision !== "allow" || parent.policy.emails.length !== 1) bad("parent policy must allow exactly one email");
if (!pub.paths.includes("/watch*") || !pub.paths.includes("/api/child*")) bad("child paths bypass");
if (pub.policy.decision !== "bypass") bad("public bypass");
if (a.apps.some((x) => x.paths.some((p) => p === "/*" || p === "/"))) bad("no catch-all app");
JS

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
