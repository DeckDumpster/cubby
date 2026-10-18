#!/usr/bin/env bash
# The cubby gate: format, lint, Rust tests, then the web check and tests. Run it from a clean
# tree, in a testenv container: `testenv --suites ci <branch> cubby`.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v npm >/dev/null 2>&1; then
    node_version=v22.22.1
    dir="$(mktemp -d)"
    curl -fsSL "https://nodejs.org/dist/${node_version}/node-${node_version}-linux-x64.tar.gz" | tar -xz -C "$dir"
    export PATH="$dir/node-${node_version}-linux-x64/bin:$PATH"
fi

echo "==> cargo fmt --check"
cargo fmt --all --check
echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
echo "==> cargo test"
cargo test --workspace

echo "==> deploy artefacts"
bash deploy/test-deploy.sh

echo "==> infra: npm ci, check, template test, cdk synth"
(cd infra && npm ci && npm run check && npm test && npx cdk synth --quiet)

cd web
echo "==> npm ci"
npm ci
echo "==> npm run check"
npm run check
echo "==> npm test"
npm test
echo "==> ci passed"
