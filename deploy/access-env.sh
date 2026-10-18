#!/usr/bin/env bash
# Sets CUBBY_ACCESS_TEAM_DOMAIN, CUBBY_ACCESS_AUD and CUBBY_ACCESS_JWKS_URL in
# ~/.config/cubby/server.env (mode 0600) from the Terraform outputs, replacing any earlier values and
# keeping every other line. deploy.sh runs it when the three are absent. Needs the Terraform backend
# reachable (an MFA session) and CLOUDFLARE_API_TOKEN set.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cfg_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cubby"
env_file="$cfg_dir/server.env"

mkdir -p "$cfg_dir"
umask 077
out() { terraform -chdir="$root/infra" output -raw "$1"; }
team="$(out access_team_domain)"
aud="$(out access_aud)"
jwks="$(out access_jwks_url)"
for v in "$team" "$aud" "$jwks"; do [ -n "$v" ] || { echo "empty access_* Terraform output" >&2; exit 1; }; done

tmp="$(mktemp "$cfg_dir/server.env.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
{
    [ ! -e "$env_file" ] || grep -vE '^CUBBY_ACCESS_(TEAM_DOMAIN|AUD|JWKS_URL)=' "$env_file" || true
    printf 'CUBBY_ACCESS_TEAM_DOMAIN=%s\nCUBBY_ACCESS_AUD=%s\nCUBBY_ACCESS_JWKS_URL=%s\n' "$team" "$aud" "$jwks"
} >"$tmp"
chmod 0600 "$tmp"
mv "$tmp" "$env_file"
trap - EXIT
echo "==> wrote CUBBY_ACCESS_* to $env_file"
