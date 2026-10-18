resource "cloudflare_zero_trust_tunnel_cloudflared" "cubby" {
  account_id = var.cloudflare_account_id
  name       = "cubby"
  config_src = "cloudflare"
}

resource "cloudflare_zero_trust_tunnel_cloudflared_config" "cubby" {
  account_id = var.cloudflare_account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.cubby.id

  config = {
    ingress = [
      {
        hostname = var.hostname
        service  = var.origin
      },
      {
        service = "http_status:404"
      },
    ]
  }
}

resource "cloudflare_dns_record" "cubby" {
  zone_id = var.cloudflare_zone_id
  name    = var.hostname
  type    = "CNAME"
  content = "${cloudflare_zero_trust_tunnel_cloudflared.cubby.id}.cfargotunnel.com"
  proxied = true
  ttl     = 1
}

resource "cloudflare_zero_trust_access_policy" "parent" {
  account_id       = var.cloudflare_account_id
  name             = "cubby-parent"
  decision         = "allow"
  session_duration = var.session_duration

  include = [for e in var.parent_emails : { email = { email = e } }]
}

resource "cloudflare_zero_trust_access_policy" "device_bypass" {
  account_id = var.cloudflare_account_id
  name       = "cubby-device-token-bypass"
  decision   = "bypass"

  include = [
    { everyone = {} },
  ]
}

resource "cloudflare_zero_trust_access_application" "parent" {
  account_id       = var.cloudflare_account_id
  name             = "cubby-parent"
  type             = "self_hosted"
  session_duration = var.session_duration

  destinations = [
    { type = "public", uri = "${var.hostname}/parent" },
    { type = "public", uri = "${var.hostname}/parent/*" },
    { type = "public", uri = "${var.hostname}/api/parent" },
  ]

  policies = [
    { id = cloudflare_zero_trust_access_policy.parent.id, precedence = 1 },
  ]
}

# /watch and /api/child are protected by the device token, not Access.
resource "cloudflare_zero_trust_access_application" "public" {
  account_id       = var.cloudflare_account_id
  name             = "cubby-public"
  type             = "self_hosted"
  session_duration = var.session_duration

  destinations = [
    { type = "public", uri = "${var.hostname}/watch" },
    { type = "public", uri = "${var.hostname}/api/child" },
    { type = "public", uri = "${var.hostname}/assets" },
    { type = "public", uri = "${var.hostname}/_app" },
    { type = "public", uri = "${var.hostname}/healthz" },
  ]

  policies = [
    { id = cloudflare_zero_trust_access_policy.device_bypass.id, precedence = 1 },
  ]
}

data "cloudflare_zero_trust_tunnel_cloudflared_token" "cubby" {
  account_id = var.cloudflare_account_id
  tunnel_id  = cloudflare_zero_trust_tunnel_cloudflared.cubby.id
}
