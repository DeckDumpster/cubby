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

DNS (`dns.tf`, `dns-records.json`): every other gantt.pub record, mirrored from Route 53 (the Fastmail
MX, SPF, DKIM CNAMEs and DMARC), unproxied. Terraform manages records only; the zone is created in the
Cloudflare dashboard and its id passed as `cloudflare_zone_id`.

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

The first plan adopts the seven existing Fastmail records in `imports.tf` instead of creating them.
Expect: `Plan: 7 to import, N to add (Cubby record, tunnel, Access), 3 to change (DKIM proxied->false), 0 to destroy`
(the three DKIM CNAMEs go proxied true -> false; MX and TXT show no change). Anything else, stop.
Once applied, `imports.tf` may be deleted.

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

## Moving gantt.pub to Cloudflare

Cloudflare Tunnel and Access need the whole zone on Cloudflare, so gantt.pub's DNS moves from Route 53.
Mail stays on Fastmail; the registrar is DreamHost.

1. Ryan creates the gantt.pub zone in the Cloudflare dashboard; note its zone id and the two assigned
   nameservers (`<name>.ns.cloudflare.com`).
2. The Concierge applies this root (see above). The records now exist on Cloudflare but are not yet live.
3. Parity check, before touching DreamHost:

       deploy/dns-diff.sh <assigned-cloudflare-nameserver>

   It asks that nameserver and Route 53 (an awsdns server found from the current NS set, or pass one as
   the second argument) for every record in `infra/dns-records.json`, and fails on any difference or any
   answer that is not the declared one. Proceed only on `dns parity ok`.
4. In DreamHost (Domains > Manage Domains > gantt.pub > DNS/Nameservers), replace the nameservers with the
   two Cloudflare ones. Leave the Route 53 zone untouched.
5. Verify mail (MX, SPF, DKIM, DMARC) and `cubby.gantt.pub` once the change propagates.

Rollback: in DreamHost, point gantt.pub back at the four `awsdns` nameservers of the Route 53 zone, which
was left in place and still answers. Nothing else needs undoing.
