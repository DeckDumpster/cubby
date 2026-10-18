# Plan-free assertions with mocked providers: no credentials, no network.
mock_provider "aws" {
  mock_data "aws_iam_policy_document" {
    defaults = {
      json = "{\"Version\":\"2012-10-17\",\"Statement\":[]}"
    }
  }
}
mock_provider "cloudflare" {}

variables {
  deploy_role_arn       = "arn:aws:iam::123456789012:role/terraform-deploy"
  cloudflare_account_id = "acct"
  cloudflare_zone_id    = "zone"
}

override_resource {
  target          = aws_s3_bucket.backups
  override_during = plan
  values = {
    arn = "arn:aws:s3:::cubby-backups"
  }
}

run "bucket_is_private_versioned_retained" {
  command = plan

  assert {
    condition     = aws_s3_bucket.backups.bucket == "cubby-backups"
    error_message = "bucket must be cubby-backups"
  }
  assert {
    condition = alltrue([
      aws_s3_bucket_public_access_block.backups.block_public_acls,
      aws_s3_bucket_public_access_block.backups.block_public_policy,
      aws_s3_bucket_public_access_block.backups.ignore_public_acls,
      aws_s3_bucket_public_access_block.backups.restrict_public_buckets,
    ])
    error_message = "all public access must be blocked"
  }
  assert {
    condition     = aws_s3_bucket_versioning.backups.versioning_configuration[0].status == "Enabled"
    error_message = "versioning must be enabled"
  }
  assert {
    condition     = one(aws_s3_bucket_server_side_encryption_configuration.backups.rule).apply_server_side_encryption_by_default[0].sse_algorithm == "AES256"
    error_message = "SSE-S3 required"
  }
  assert {
    condition     = one(aws_s3_bucket_lifecycle_configuration.backups.rule).noncurrent_version_expiration[0].noncurrent_days == 30
    error_message = "noncurrent versions must expire after 30 days"
  }
  assert {
    condition     = one(data.aws_iam_policy_document.tls_only.statement).effect == "Deny" && one([for c in one(data.aws_iam_policy_document.tls_only.statement).condition : c.variable]) == "aws:SecureTransport"
    error_message = "bucket policy must deny non-TLS"
  }
}

run "iam_policy_names_only_this_bucket" {
  command = plan

  assert {
    condition     = length(data.aws_iam_policy_document.backup.statement) == 2
    error_message = "exactly two statements"
  }
  assert {
    condition = alltrue(flatten([
      for s in data.aws_iam_policy_document.backup.statement :
      [for r in s.resources : r == "arn:aws:s3:::cubby-backups" || r == "arn:aws:s3:::cubby-backups/*"]
    ]))
    error_message = "policy resources must be this bucket only"
  }
  assert {
    condition = toset(flatten([for s in data.aws_iam_policy_document.backup.statement : s.actions])) == toset([
      "s3:ListBucket", "s3:GetObject", "s3:PutObject", "s3:DeleteObject",
    ])
    error_message = "actions must be List/Get/Put/Delete only"
  }
  assert {
    condition     = aws_secretsmanager_secret.litestream.name == "cubby/litestream"
    error_message = "secret name"
  }
}

run "access_allows_only_ryan" {
  command = plan

  assert {
    condition     = cloudflare_zero_trust_access_policy.parent.decision == "allow" && length(cloudflare_zero_trust_access_policy.parent.include) == 1 && one(cloudflare_zero_trust_access_policy.parent.include).email.email == "ryan@gantt.pub"
    error_message = "parent policy must allow exactly ryan@gantt.pub"
  }
  assert {
    condition     = cloudflare_zero_trust_access_policy.device_bypass.decision == "bypass"
    error_message = "device bypass policy"
  }
  assert {
    condition     = toset([for d in cloudflare_zero_trust_access_application.parent.destinations : d.uri]) == toset(["cubby.gantt.pub/parent", "cubby.gantt.pub/api/parent"])
    error_message = "parent app covers /parent and /api/parent"
  }
  assert {
    condition     = contains([for d in cloudflare_zero_trust_access_application.public.destinations : d.uri], "cubby.gantt.pub/watch") && !contains([for d in cloudflare_zero_trust_access_application.public.destinations : d.uri], "cubby.gantt.pub")
    error_message = "bypass app covers /watch and never the whole host"
  }
  assert {
    condition     = cloudflare_zero_trust_tunnel_cloudflared_config.cubby.config.ingress[0].service == "http://127.0.0.1:8790" && cloudflare_zero_trust_tunnel_cloudflared_config.cubby.config.ingress[1].service == "http_status:404"
    error_message = "tunnel ingress"
  }
}

run "iam_principals_carry_the_deploy_boundary" {
  command = plan

  assert {
    condition     = aws_iam_user.backup.permissions_boundary == "arn:aws:iam::189923011121:policy/terraform-deploy-boundary"
    error_message = "every aws_iam_user must set the terraform-deploy-boundary permissions boundary"
  }
}

run "fastmail_dns_is_declared_exactly_and_unproxied" {
  command = plan

  assert {
    condition = toset([
      for r in values(cloudflare_dns_record.zone) : "${r.name} ${r.type} ${r.priority == null ? "" : r.priority} ${r.content}"
      ]) == toset([
      "gantt.pub MX 10 in1-smtp.messagingengine.com",
      "gantt.pub MX 20 in2-smtp.messagingengine.com",
      "gantt.pub TXT  \"v=spf1 include:spf.messagingengine.com ?all\"",
      "fm1._domainkey.gantt.pub CNAME  fm1.gantt.pub.dkim.fmhosted.com",
      "fm2._domainkey.gantt.pub CNAME  fm2.gantt.pub.dkim.fmhosted.com",
      "fm3._domainkey.gantt.pub CNAME  fm3.gantt.pub.dkim.fmhosted.com",
      "_dmarc.gantt.pub TXT  \"v=DMARC1; p=none;\"",
    ])
    error_message = "every Fastmail record must be present exactly"
  }
  assert {
    condition     = alltrue([for r in values(cloudflare_dns_record.zone) : r.proxied == false])
    error_message = "mail records must be unproxied"
  }
  assert {
    condition     = cloudflare_dns_record.cubby.proxied == true
    error_message = "cubby's own record is proxied"
  }
}
