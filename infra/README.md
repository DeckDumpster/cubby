# Cubby infrastructure (CDK)

One stack, `CubbyBackups` (TypeScript, `aws-cdk-lib` pinned exact):

- S3 bucket `cubby-backups`: all public access blocked, SSE-S3, TLS-only, versioned, noncurrent
  versions expire after 30 days, `RemovalPolicy.RETAIN`.
- IAM user whose only policy is `s3:ListBucket` on the bucket and `s3:GetObject`/`PutObject`/`DeleteObject`
  on its objects, with an access key stored in Secrets Manager as `cubby/litestream`
  (JSON: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`).

## Check (no AWS credentials)

    cd infra && npm ci && npm test && npx cdk synth --quiet

`deploy/ci.sh` runs this, including the template assertions in `test/template.test.ts`.

## Deploy (the Concierge, with an MFA session)

    cd infra && npm ci && npx cdk deploy CubbyBackups

Requires the account to be `cdk bootstrap`ped. Never deployed from CI.

## Outputs

| Output | Use |
| --- | --- |
| `BucketName` | `cubby-backups`, hardcoded in `deploy/litestream.yml` |
| `BucketRegion` | `LITESTREAM_S3_REGION` in `litestream.env` |
| `CredentialSecretName` | `cubby/litestream`, the secret holding the access key |

## Writing litestream.env on the box

    LITESTREAM_S3_REGION=<BucketRegion> deploy/litestream-env.sh

This reads the secret with the aws CLI and writes `~/.config/cubby/litestream.env` mode 0600;
then run `deploy/deploy.sh`.
