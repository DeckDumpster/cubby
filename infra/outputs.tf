output "bucket_name" {
  value = aws_s3_bucket.backups.bucket
}

output "bucket_region" {
  value = var.aws_region
}

output "credential_secret_name" {
  value = aws_secretsmanager_secret.litestream.name
}

output "tunnel_id" {
  value = cloudflare_zero_trust_tunnel_cloudflared.cubby.id
}

output "tunnel_token" {
  value     = data.cloudflare_zero_trust_tunnel_cloudflared_token.cubby.token
  sensitive = true
}
