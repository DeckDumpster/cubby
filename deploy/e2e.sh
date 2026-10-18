#!/usr/bin/env bash
# The whole-product walk in a real browser (web/e2e-full/walk.spec.js): the cubby-server binary
# serving the built web app, Access stubbed by a local JWKS, YouTube by a fake Data API. Part of the
# gate (deploy/ci.sh), and runnable alone: `testenv --suites e2e <branch> cubby`.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
# shellcheck source=deploy/ensure-node.sh
source "$root/deploy/ensure-node.sh"

echo "==> cargo build cubby-server"
cargo build -p cubby-server
# The harness finds the binary under CARGO_TARGET_DIR (default ./target).

cd web
echo "==> npm ci, build"
npm ci
npm run build
echo "==> playwright chromium"
SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO=sudo
$SUDO env "PATH=$PATH" npx playwright install --with-deps chromium
echo "==> walk"
npm run e2e:full
