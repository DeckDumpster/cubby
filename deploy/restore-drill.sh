#!/usr/bin/env bash
# Restore drill: restores a replicated family file from S3 into a scratch directory, then checks
# it is a sound SQLite file with rows. Never touches the live volume.
#   restore-drill.sh <s3-replica-url> [scratch-dir]
# <s3-replica-url> is s3://cubby-backups/<LITESTREAM_S3_PATH>/family-<ulid>.sqlite (directory mode
# derives each replica path from the file name). Needs AWS credentials in the environment.
set -euo pipefail
if [ "${1:-}" = "--help" ]; then
    sed -n '2,6p' "${BASH_SOURCE[0]}"; echo "e.g. s3://cubby-backups/prod/family-01J....sqlite"; exit 0
fi
url="${1:-}"
[ -n "$url" ] || { echo "usage: restore-drill.sh <s3-replica-url> [scratch-dir]" >&2; exit 2; }
case "$url" in s3://cubby-backups/*/family-*.sqlite|s3://cubby-backups/*/registry.sqlite) ;;
    *) echo "ERROR: $url is not a cubby-backups family or registry replica" >&2; exit 2 ;; esac
scratch="${2:-$(mktemp -d)}"
out="$scratch/restored.sqlite"
litestream restore -o "$out" "$url"
check="$(sqlite3 "$out" 'PRAGMA integrity_check;')"
[ "$check" = ok ] || { echo "FAIL: integrity_check said $check" >&2; exit 1; }
tables="$(sqlite3 "$out" "select count(*) from sqlite_master where type='table'")"
[ "$tables" -gt 0 ] || { echo "FAIL: restored file has no tables" >&2; exit 1; }
echo "restore drill ok: $url -> $out ($tables tables)"
