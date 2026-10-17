#!/usr/bin/env bash
# Applies deploy/cloudflare/access.json and the tunnel DNS route through the Cloudflare API.
# Idempotent: apps are found by name and PUT, created only when absent.
# Needs, from the environment: CF_API_TOKEN (Access + DNS + Tunnel edit), CF_ACCOUNT_ID, CF_ZONE_ID,
# CF_TUNNEL_ID. Needs the gantt.pub zone on Cloudflare first (Ryan's step); refuses without it.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for v in CF_API_TOKEN CF_ACCOUNT_ID CF_ZONE_ID CF_TUNNEL_ID; do
    [ -n "${!v:-}" ] || { echo "ERROR: $v is not set (see deploy/README or the cutover ask)" >&2; exit 2; }
done
api=https://api.cloudflare.com/client/v4
call() { curl -fsS -H "Authorization: Bearer $CF_API_TOKEN" -H 'Content-Type: application/json' "$@"; }

host="$(jq -r .hostname "$here/access.json")"
dur="$(jq -r .session_duration "$here/access.json")"

# DNS: cname to the tunnel, created or updated in place.
target="$CF_TUNNEL_ID.cfargotunnel.com"
rec="$(call "$api/zones/$CF_ZONE_ID/dns_records?name=$host" | jq -r '.result[0].id // empty')"
body="$(jq -n --arg n "$host" --arg t "$target" '{type:"CNAME",name:$n,content:$t,proxied:true}')"
if [ -n "$rec" ]; then call -X PUT -d "$body" "$api/zones/$CF_ZONE_ID/dns_records/$rec" >/dev/null
else call -X POST -d "$body" "$api/zones/$CF_ZONE_ID/dns_records" >/dev/null; fi

# One Access app per path set (an app per path, domain = host+path).
existing="$(call "$api/accounts/$CF_ACCOUNT_ID/access/apps")"
jq -c '.apps[]' "$here/access.json" | while read -r app; do
    name="$(jq -r .name <<<"$app")"
    domains="$(jq -c --arg h "$host" '[.paths[] | $h + .]' <<<"$app")"
    pol="$(jq -c '.policy' <<<"$app")"
    appbody="$(jq -n --arg n "$name" --arg d "$dur" --argjson doms "$domains" \
        '{name:$n,type:"self_hosted",session_duration:$d,destinations:[$doms[]|{type:"public",uri:.}],domain:$doms[0]}')"
    id="$(jq -r --arg n "$name" '.result[]|select(.name==$n)|.id' <<<"$existing" | head -n1)"
    if [ -n "$id" ]; then call -X PUT -d "$appbody" "$api/accounts/$CF_ACCOUNT_ID/access/apps/$id" >/dev/null
    else id="$(call -X POST -d "$appbody" "$api/accounts/$CF_ACCOUNT_ID/access/apps" | jq -r .result.id)"; fi
    decision="$(jq -r .decision <<<"$pol")"
    include="$(jq -c 'if .decision=="bypass" then [{everyone:{}}] else [.emails[]|{email:{email:.}}] end' <<<"$pol")"
    polbody="$(jq -n --arg n "$(jq -r .name <<<"$pol")" --arg d "$decision" --argjson inc "$include" \
        '{name:$n,decision:$d,include:$inc,precedence:1}')"
    pid="$(call "$api/accounts/$CF_ACCOUNT_ID/access/apps/$id/policies" | jq -r '.result[0].id // empty')"
    if [ -n "$pid" ]; then call -X PUT -d "$polbody" "$api/accounts/$CF_ACCOUNT_ID/access/apps/$id/policies/$pid" >/dev/null
    else call -X POST -d "$polbody" "$api/accounts/$CF_ACCOUNT_ID/access/apps/$id/policies" >/dev/null; fi
    echo "access app $name ok"
done
