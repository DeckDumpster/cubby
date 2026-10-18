#!/usr/bin/env bash
# Writes ~/.config/cubby/cloudflared.env (mode 0600) holding TUNNEL_TOKEN, read from the Terraform
# output at run time. The token is never printed or committed. Run it from the production checkout
# with the Terraform backend reachable (an MFA session) and CLOUDFLARE_API_TOKEN set.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cfg_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cubby"

mkdir -p "$cfg_dir"
umask 077
tmp="$(mktemp "$cfg_dir/cloudflared.env.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
token="$(terraform -chdir="$root/infra" output -raw tunnel_token)"
[ -n "$token" ] || { echo "empty tunnel_token output" >&2; exit 1; }
printf 'TUNNEL_TOKEN=%s\n' "$token" >"$tmp"
chmod 0600 "$tmp"
mv "$tmp" "$cfg_dir/cloudflared.env"
trap - EXIT
echo "==> wrote $cfg_dir/cloudflared.env"
