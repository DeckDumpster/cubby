#!/usr/bin/env bash
# The cubby gate: format, lint, Rust tests, then the web check and tests. Run it from a clean
# tree, in a testenv container: `testenv --suites ci <branch> cubby`.
set -euo pipefail

# `--build-only` compiles everything the gate tests and runs no test, so the harness can time
# only the tests against its cap: `testenv --suites build <branch> cubby`.
build_only=0
case "${1-}" in
    "") ;;
    --build-only) [ "$#" -eq 1 ] && build_only=1 ;;
esac
if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$build_only" -eq 0 ]; }; then
    echo "usage: deploy/ci.sh [--build-only]" >&2
    exit 2
fi

# Every non-Rust tool a check in the gate uses goes on this list; the image has none of them.
# A new check that calls a new tool adds it here, or it goes red only in the round.
gate_tools=(jq dnsutils)

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [ "$build_only" -eq 1 ]; then
    echo "==> build only: cargo clippy"
    cargo clippy --workspace --all-targets -- -D warnings
    echo "==> build only: cargo test --no-run"
    cargo test --workspace --no-run
    echo "==> build only: cubby-server"
    cargo build -p cubby-server
    echo "==> build passed"
    exit 0
fi

missing=()
for t in "${gate_tools[@]}"; do
    dpkg -s "$t" >/dev/null 2>&1 || missing+=("$t")
done
if [ "${#missing[@]}" -gt 0 ]; then
    SUDO=""
    [ "$(id -u)" -eq 0 ] || SUDO=sudo
    $SUDO apt-get update -qq
    $SUDO apt-get install -y -qq "${missing[@]}"
fi

source "$root/deploy/ensure-node.sh"

echo "==> cargo fmt --check"
cargo fmt --all --check
echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
echo "==> cargo test"
cargo test --workspace

echo "==> deploy artefacts"
bash deploy/test-deploy.sh
bash deploy/test-dns-diff.sh

echo "==> infra: terraform fmt, validate, test"
tf_version=1.13.3
tf_sha256=71fc43d92ea09907be5d416d2405a6a9c2d1ceaed633f5e175c0af26e8c4b365
if ! command -v terraform >/dev/null 2>&1; then
    dir="$(mktemp -d)"
    curl -fsSL -o "$dir/tf.zip" "https://releases.hashicorp.com/terraform/${tf_version}/terraform_${tf_version}_linux_amd64.zip"
    echo "${tf_sha256}  $dir/tf.zip" | sha256sum -c -
    python3 -c 'import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extract("terraform", sys.argv[2])' "$dir/tf.zip" "$dir"
    chmod +x "$dir/terraform"
    export PATH="$dir:$PATH"
fi
(
    cd infra
    terraform fmt -check -recursive
    { terraform init -backend=false -input=false || { sleep 5; terraform init -backend=false -input=false; }; }
    terraform validate
    terraform test
)

cd web
echo "==> npm ci"
npm ci
echo "==> npm run check"
npm run check
echo "==> npm test"
npm test
cd ..
echo "==> end-to-end walk"
bash deploy/e2e.sh
echo "==> ci passed"
