# Every gantt.pub record other than Cubby's own (cloudflare.tf), mirrored from Route 53 for the
# nameserver move. Data lives in dns-records.json so deploy/dns-diff.sh reads the same list.
# The zone itself is created in the dashboard; Terraform manages records only.
locals {
  zone_records = {
    for r in jsondecode(file("${path.module}/dns-records.json")) :
    "${r.name}/${r.type}/${try(r.priority, "")}/${r.content}" => r
  }
}

resource "cloudflare_dns_record" "zone" {
  for_each = local.zone_records

  zone_id  = var.cloudflare_zone_id
  name     = each.value.name
  type     = each.value.type
  content  = each.value.type == "TXT" ? "\"${each.value.content}\"" : each.value.content
  priority = try(each.value.priority, null)
  ttl      = each.value.ttl
  proxied  = false
}
