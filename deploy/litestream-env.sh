#!/usr/bin/env bash
# Writes ~/.config/cubby/litestream.env (mode 0600) from the Secrets Manager secret the
# CubbyBackups stack creates (infra/README.md). Needs the aws CLI and read access to the secret.
set -euo pipefail

cfg_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cubby"
secret="${CUBBY_BACKUPS_SECRET:-cubby/litestream}"
region="${LITESTREAM_S3_REGION:?set LITESTREAM_S3_REGION to the bucket_region Terraform output}"
path="${LITESTREAM_S3_PATH:-cubby}"

mkdir -p "$cfg_dir"
umask 077
tmp="$(mktemp "$cfg_dir/litestream.env.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
aws secretsmanager get-secret-value --region "$region" --secret-id "$secret" --query SecretString --output text \
    | node -e 'const s=JSON.parse(require("fs").readFileSync(0,"utf8"));for(const k of ["AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY"])console.log(k+"="+s[k])' >"$tmp"
printf 'LITESTREAM_S3_PATH=%s\nLITESTREAM_S3_REGION=%s\n' "$path" "$region" >>"$tmp"
chmod 0600 "$tmp"
mv "$tmp" "$cfg_dir/litestream.env"
trap - EXIT
echo "==> wrote $cfg_dir/litestream.env"
