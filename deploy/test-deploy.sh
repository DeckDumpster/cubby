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

# --- Deploy refuses on a host that is not the declared prod host, before touching anything.
[ "$(tr -d '[:space:]' < prod-host)" = ubuntu-mini-0 ] || fail "prod-host is not ubuntu-mini-0"
rh="$(mktemp -d)"
if out="$(CUBBY_DEPLOY_HOSTNAME=agent-swarm-test XDG_CONFIG_HOME="$rh" ./deploy.sh 2>&1)"; then fail "deploy ran on a non-prod host"; fi
case "$out" in *"Cubby production is 'ubuntu-mini-0'"*) ;; *) fail "refusal does not name the reason: $out";; esac
[ -z "$(ls -A "$rh")" ] || fail "refused deploy still wrote files"
rm -rf "$rh"
export CUBBY_DEPLOY_HOSTNAME=ubuntu-mini-0

# --- push-prod.sh connects by the tailnet name by default, not the bare hostname.
[ "$(env -u CUBBY_PROD_SSH CUBBY_PUSH_PRINT_TARGET=1 ./push-prod.sh)" = ryangantt@ubuntu-mini-0.tailb4a728.ts.net ] || fail "push-prod default destination is not the tailnet name"

# --- Access settings: an empty server.env with no Terraform makes deploy refuse, naming all three;
# a deploy from scratch with Terraform outputs available ends with them set.
a="$(mktemp -d)"; fakebin="$(mktemp -d)"
printf '#!/bin/sh\nexit 1\n' >"$fakebin/terraform"; chmod +x "$fakebin/terraform"
if msg="$(PATH="$fakebin:$PATH" CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$a" ./deploy.sh 2>&1)"; then fail "deploy succeeded with empty server.env"; fi
for v in CUBBY_ACCESS_TEAM_DOMAIN CUBBY_ACCESS_AUD CUBBY_ACCESS_JWKS_URL; do
    case "$msg" in *"$v"*) ;; *) fail "refusal does not name $v: $msg";; esac
done
cat >"$fakebin/terraform" <<'T'
#!/bin/sh
case "$*" in
*access_team_domain) printf 'https://t.cloudflareaccess.com';;
*access_aud) printf 'aud123';;
*access_jwks_url) printf 'https://t.cloudflareaccess.com/cdn-cgi/access/certs';;
*) exit 1;;
esac
T
printf 'CUBBY_PORT=1\nCUBBY_ACCESS_AUD=stale\n' >"$a/cubby/server.env"
PATH="$fakebin:$PATH" CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$a" ./deploy.sh >/dev/null
grep -qx 'CUBBY_ACCESS_AUD=aud123' "$a/cubby/server.env" || fail "aud not written"
grep -qx 'CUBBY_ACCESS_TEAM_DOMAIN=https://t.cloudflareaccess.com' "$a/cubby/server.env" || fail "team not written"
grep -q '^CUBBY_ACCESS_JWKS_URL=.*certs$' "$a/cubby/server.env" || fail "jwks not written"
grep -qx 'CUBBY_PORT=1' "$a/cubby/server.env" || fail "other settings dropped"
[ "$(grep -c AUD "$a/cubby/server.env")" = 1 ] || fail "stale aud kept"
rm -rf "$a" "$fakebin"

# --- Deploy is idempotent: running the install step twice leaves identical files.
t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
mkdir -p "$t/cubby"
printf 'CUBBY_ACCESS_TEAM_DOMAIN=x\nCUBBY_ACCESS_AUD=y\nCUBBY_ACCESS_JWKS_URL=z\n' >"$t/cubby/server.env"
for i in 1 2; do CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null; done
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum1"
CUBBY_DEPLOY_DRY_RUN=1 XDG_CONFIG_HOME="$t" ./deploy.sh >/dev/null
( cd "$t" && find . -type f | sort | xargs sha256sum ) > "$t.sum2"
cmp "$t.sum1" "$t.sum2" || fail "deploy not idempotent"
[ -f "$t/containers/systemd/cubby-litestream.container" ] || fail "litestream unit not installed"
[ -f "$t/containers/systemd/cubby-cloudflared.container" ] || fail "cloudflared unit not installed"
rm -f "$t.sum1" "$t.sum2"
[ -f "$t/cubby/server.env" ] || fail "server.env not created"
[ -f "$t/cubby/worker.env" ] || fail "worker.env not created"

# --- Every env file a unit names must be created by deploy, or deploy must skip the unit without it.
for f in *.container *.service; do
    for name in $(grep -hoE '(EnvironmentFile=|--env-file=)[^ ]*' "$f" | sed 's|.*/||'); do
        [ -f "$t/cubby/$name" ] && continue
        grep -q "\[ -f \"\$cfg_dir/$name\" \]" deploy.sh || fail "$f names $name: deploy neither creates nor guards it"
    done
done
echo "deploy artefacts ok"
