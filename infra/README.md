# Cubby infrastructure (Terraform)

One Terraform root (HashiCorp Terraform >= 1.10, `hashicorp/aws` and `cloudflare/cloudflare` pinned
exactly in `versions.tf`). State is in the shared state bucket with S3-native locking
(`use_lockfile`), reached by assuming the `terraform-deploy` role (sp-lgwrfq).

AWS (`backups.tf`):

- S3 bucket `cubby-backups`: public access blocked, SSE-S3, TLS-only, versioned, noncurrent versions
  expire after 30 days, `prevent_destroy`.
- IAM user `cubby-litestream` whose only policy is `s3:ListBucket` on the bucket and
  `s3:GetObject`/`PutObject`/`DeleteObject` on its objects, with an access key stored in Secrets
  Manager as `cubby/litestream` (JSON: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`).

Cloudflare (`cloudflare.tf`): the `cubby.gantt.pub` CNAME, the `cubby` tunnel with ingress to
`http://127.0.0.1:8790` (404 catch-all), an Access application on `/parent` and `/api/parent`
allowing only `ryan@gantt.pub`, and a bypass application for `/watch`, `/api/child`, `/assets`,
`/_app` and `/healthz` (protected by the device token instead).

## Check (no credentials)

    cd infra && terraform fmt -check -recursive && terraform init -backend=false \
      && terraform validate && terraform test

`deploy/ci.sh` runs this; `tests/guarantees.tftest.hcl` uses mocked providers to assert the bucket is
private/versioned/retained, the IAM policy names only this bucket, and Access allows only ryan@gantt.pub.

## Plan and apply (the Concierge, with an MFA session and a Cloudflare token)

    export CLOUDFLARE_API_TOKEN=...   # Access, DNS and Tunnel edit
    cd infra
    terraform init -backend-config="bucket=<StateBucketName>" \
      -backend-config="assume_role={role_arn=\"<DeployRoleArn>\"}"
    terraform plan  -out=cubby.tfplan -var deploy_role_arn=<DeployRoleArn> \
      -var cloudflare_account_id=<id> -var cloudflare_zone_id=<gantt.pub zone id>
    terraform apply cubby.tfplan

Never applied from CI. Note: the `terraform-deploy` role is scoped to `spira-*` names; `cubby-backups`,
`cubby-litestream` and `cubby/litestream` need the role widened first.

## Outputs

| Output | Use |
| --- | --- |
| `bucket_name` | `cubby-backups`, hardcoded in `deploy/litestream.yml` |
| `bucket_region` | `LITESTREAM_S3_REGION` in `litestream.env` |
| `credential_secret_name` | `cubby/litestream`, the secret holding the access key |
| `tunnel_id` | the `cubby` tunnel |
| `tunnel_token` (sensitive) | `terraform output -raw tunnel_token`, for `cloudflared tunnel run --token` |

## Writing litestream.env on the box

    LITESTREAM_S3_REGION=$(terraform output -raw bucket_region) deploy/litestream-env.sh

Then run `deploy/deploy.sh`.
