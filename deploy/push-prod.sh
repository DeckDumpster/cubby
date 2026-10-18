#!/usr/bin/env bash
# Ships the landing ref to the production checkout (~/cubby-prod on the host in deploy/prod-host) as
# a git bundle over ssh, then runs the deploy there. Cubby has no remote; this is the transport.
# Run it from the Spira checkout of cubby. Env: CUBBY_PROD_SSH (default ryangantt@<prod-host>),
# CUBBY_LANDING_REF (default local/main).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host="$(tr -d '[:space:]' < "$root/deploy/prod-host")"
target="${CUBBY_PROD_SSH:-ryangantt@$host}"
ref="${CUBBY_LANDING_REF:-local/main}"

bundle="$(mktemp -d)/cubby.bundle"
git -C "$root" bundle create "$bundle" "$ref"
scp -q "$bundle" "$target:/tmp/cubby.bundle"
ssh "$target" "set -euo pipefail
[ -d ~/cubby-prod/.git ] || git init -q ~/cubby-prod
git -C ~/cubby-prod fetch -q /tmp/cubby.bundle '$ref':refs/heads/landed --force
git -C ~/cubby-prod checkout -q -f -B main landed
rm -f /tmp/cubby.bundle
~/cubby-prod/deploy/deploy.sh"
