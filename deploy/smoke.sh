#!/usr/bin/env bash
# The user-path walk (web/e2e-full/walk.spec.js) against the deployed instance, in the Playwright
# image: /parent sends a signed-out browser to sign in at the provider, the child routes are mounted, and
# a paired device sees its catalogue. deploy.sh runs it last and reports the deploy failed if it
# does. The step that needs a credential is skipped, with the reason printed, when it is absent:
# put SMOKE_DEVICE_TOKEN (a paired device's token) in <config dir>/cubby/smoke.env.
#
# Env: CUBBY_SMOKE_URL (default http://127.0.0.1:8790), CUBBY_SMOKE_WAIT_S (how long to wait for
# /healthz, default 60).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cfg_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cubby"
url="${CUBBY_SMOKE_URL:-http://127.0.0.1:8790}"
wait_s="${CUBBY_SMOKE_WAIT_S:-60}"
playwright_version="$(sed -n 's/.*"@playwright\/test": "\([0-9.]*\)".*/\1/p' "$root/web/package.json")"
[ -n "$playwright_version" ] || { echo "smoke: no pinned @playwright/test in web/package.json" >&2; exit 1; }
image="mcr.microsoft.com/playwright:v${playwright_version}-noble"

echo "==> smoke: waiting up to ${wait_s}s for $url/healthz"
for ((i = 0; i < wait_s; i++)); do
    curl -fsS "$url/healthz" >/dev/null 2>&1 && break
    sleep 1
done
curl -fsS "$url/healthz" >/dev/null || { echo "==> smoke: $url/healthz never answered" >&2; exit 1; }

env_file="$(mktemp)"
trap 'rm -f "$env_file"' EXIT
chmod 600 "$env_file"
echo "SMOKE_BASE_URL=$url" >"$env_file"
[ -f "$cfg_dir/smoke.env" ] && cat "$cfg_dir/smoke.env" >>"$env_file"

echo "==> smoke: walking $url with $image"
podman run --rm --network=host --env-file "$env_file" -v "$root/web:/web:Z" -w /web "$image" \
    bash -c 'npm ci --no-audit --no-fund && npx playwright test -c playwright.smoke.config.js'
echo "==> smoke: passed"
