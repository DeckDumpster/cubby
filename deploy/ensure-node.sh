# Sourced by the gate scripts: puts node and npm on PATH, fetching a pinned node when the image has none.
if ! command -v npm >/dev/null 2>&1; then
    node_version=v22.22.1
    dir="$(mktemp -d)"
    curl -fsSL "https://nodejs.org/dist/${node_version}/node-${node_version}-linux-x64.tar.gz" | tar -xz -C "$dir"
    export PATH="$dir/node-${node_version}-linux-x64/bin:$PATH"
fi
