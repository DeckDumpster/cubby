# The seven Fastmail records already exist in the gantt.pub zone (made in the dashboard), so they
# are adopted into cloudflare_dns_record.zone rather than created. Keys are local.zone_records' keys
# ("name/type/priority/content"); ids were read from the live zone on 2026-10-18.
locals {
  zone_record_ids = {
    "fm1._domainkey.gantt.pub/CNAME//fm1.gantt.pub.dkim.fmhosted.com" = "23d706d68a716268dd8745dbcacf9924"
    "fm2._domainkey.gantt.pub/CNAME//fm2.gantt.pub.dkim.fmhosted.com" = "65016fa6e9830e868bbd582c92a1030f"
    "fm3._domainkey.gantt.pub/CNAME//fm3.gantt.pub.dkim.fmhosted.com" = "fc332cdbf2814c6fc9a254d2bf351ca0"
    "gantt.pub/MX/20/in2-smtp.messagingengine.com"                    = "b9e56acf1f4835dfb9f7d74b9ec940ec"
    "gantt.pub/MX/10/in1-smtp.messagingengine.com"                    = "7874481bcff875236a5d3ad93b3a83f0"
    "_dmarc.gantt.pub/TXT//v=DMARC1; p=none;"                         = "f40233630bb69cf50333a0cf4256132f"
    "gantt.pub/TXT//v=spf1 include:spf.messagingengine.com ?all"      = "704a9df033b057673f763a24a9d33061"
  }
}

import {
  for_each = local.zone_record_ids
  to       = cloudflare_dns_record.zone[each.key]
  id       = "${var.cloudflare_zone_id}/${each.value}"
}
