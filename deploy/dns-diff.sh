#!/usr/bin/env bash
# Prove DNS parity before gantt.pub's nameservers move from Route 53 to Cloudflare.
# Queries <target-ns> and Route 53 for every record in infra/dns-records.json (the list
# Terraform applies) and fails on any difference, or on any answer that is not the declared one.
#   deploy/dns-diff.sh <cloudflare-ns> [<route53-ns>]
# route53-ns defaults to one of gantt.pub's current awsdns nameservers. DNS_DIFF_DIG overrides dig (tests).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
records="${DNS_DIFF_RECORDS:-$here/../infra/dns-records.json}"
dig_bin="${DNS_DIFF_DIG:-dig}"
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: dns-diff.sh <target-nameserver> [<route53-nameserver>]" >&2; exit 2; }
target="$1"
zone="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[0]["name"])' "$records" | awk -F. '{print $(NF-1)"."$NF}')"

r53="${2:-}"
if [ -z "$r53" ]; then
    r53="$("$dig_bin" +short NS "$zone" | grep -i 'awsdns' | head -n1 | sed 's/\.$//' || true)"
    [ -n "$r53" ] || { echo "FAIL: no awsdns nameserver found for $zone; pass the Route 53 nameserver as the second argument" >&2; exit 2; }
fi

# One normalised line per answer: lowercase, no trailing dot, no TXT quotes, sorted.
norm() { tr 'A-Z' 'a-z' | sed -e 's/" "//g' -e 's/"//g' -e 's/\.$//' -e 's/\. / /' | sed '/^$/d' | sort; }
ask() { "$dig_bin" +short +norecurse "@$1" "$3" "$2" | norm; }

fail=0
while IFS=$'\t' read -r name type want; do
    expect="$(printf '%s\n' "${want//|/$'\n'}" | norm)"
    got_t="$(ask "$target" "$type" "$name")"
    got_r="$(ask "$r53" "$type" "$name")"
    if [ "$got_t" != "$got_r" ]; then
        echo "DIFF $name $type"
        echo "  $target:"; printf '%s\n' "$got_t" | sed 's/^/    /'
        echo "  $r53:"; printf '%s\n' "$got_r" | sed 's/^/    /'
        fail=1
    fi
    if [ "$got_t" != "$expect" ]; then
        echo "UNDECLARED $name $type: $target answers [${got_t//$'\n'/, }], Terraform declares [${expect//$'\n'/, }]"
        fail=1
    fi
done < <(python3 -I -c '
import json, sys
groups = {}
for r in json.load(open(sys.argv[1])):
    c = "%s %s" % (r["priority"], r["content"]) if r.get("priority") else r["content"]
    groups.setdefault((r["name"], r["type"]), []).append(c)
for (n, t), cs in sorted(groups.items()):
    print("\t".join([n, t, "|".join(cs)]))
' "$records")
[ "$fail" = 0 ] && echo "dns parity ok: $target matches $r53 for all records in $(basename "$records")"
exit "$fail"
