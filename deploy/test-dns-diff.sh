#!/usr/bin/env bash
# dns-diff.sh against fixture answers (a fake dig reading files per nameserver). Runs from deploy/ci.sh.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
fail() { echo "FAIL: $*" >&2; exit 1; }
t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
mkdir "$t/r53" "$t/cf"
cat > "$t/dig" <<'DIG'
#!/usr/bin/env bash
# dig +short [+norecurse] [@ns] name TYPE  |  dig +short NS zone
ns=""; args=()
for a in "$@"; do case "$a" in +*) ;; @*) ns="${a#@}";; *) args+=("$a");; esac; done
if [ -z "$ns" ]; then printf 'ns-1.awsdns-01.org.\nns-2.awsdns-02.com.\n'; exit 0; fi
f="$FIXTURES/$ns/${args[0]}_${args[1]}"
[ -f "$f" ] && cat "$f" || true
DIG
chmod +x "$t/dig"
export DNS_DIFF_DIG="$t/dig" FIXTURES="$t"
w() { mkdir -p "$t/$1"; printf '%s\n' "${@:4}" > "$t/$1/$2_$3"; }
fill() {
    w "$1" gantt.pub MX '10 in1-smtp.messagingengine.com.' '20 in2-smtp.messagingengine.com.'
    w "$1" gantt.pub TXT '"v=spf1 include:spf.messagingengine.com ?all"'
    for n in 1 2 3; do w "$1" "fm$n._domainkey.gantt.pub" CNAME "fm$n.gantt.pub.dkim.fmhosted.com."; done
    w "$1" _dmarc.gantt.pub TXT '"v=DMARC1; p=none;"'
}
fill ns-1.awsdns-01.org; fill cf

out="$(./dns-diff.sh cf)" || fail "identical zones reported a diff: $out"
case "$out" in *"parity ok"*) ;; *) fail "no ok line: $out";; esac

w cf gantt.pub MX '10 in1-smtp.messagingengine.com.'
if out="$(./dns-diff.sh cf ns-1.awsdns-01.org)"; then fail "missing MX not caught"; fi
case "$out" in *"DIFF gantt.pub MX"*) ;; *) fail "MX diff not reported: $out";; esac

fill cf; w cf _dmarc.gantt.pub TXT '"v=DMARC1; p=none;"' ; w ns-1.awsdns-01.org _dmarc.gantt.pub TXT '"v=DMARC1; p=reject;"'
if out="$(./dns-diff.sh cf)"; then fail "changed DMARC not caught"; fi
case "$out" in *"DIFF _dmarc.gantt.pub TXT"*) ;; *) fail "DMARC diff not reported: $out";; esac

# Both servers agree but differ from what Terraform declares.
fill cf; fill ns-1.awsdns-01.org
w cf fm1._domainkey.gantt.pub CNAME other.example.; w ns-1.awsdns-01.org fm1._domainkey.gantt.pub CNAME other.example.
if out="$(./dns-diff.sh cf)"; then fail "undeclared answer not caught"; fi
case "$out" in *"UNDECLARED fm1._domainkey.gantt.pub"*) ;; *) fail "undeclared not reported: $out";; esac
echo "dns-diff ok"
