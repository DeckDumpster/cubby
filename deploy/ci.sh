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
echo "==> ci passed"
