import { CfnOutput, Duration, RemovalPolicy, SecretValue, Stack, type StackProps } from "aws-cdk-lib";
import * as iam from "aws-cdk-lib/aws-iam";
import * as s3 from "aws-cdk-lib/aws-s3";
import * as secretsmanager from "aws-cdk-lib/aws-secretsmanager";
import type { Construct } from "constructs";

export const BUCKET_NAME = "cubby-backups";

/** The Litestream replica bucket and the one credential allowed to write to it. */
export class CubbyBackups extends Stack {
	constructor(scope: Construct, id: string, props?: StackProps) {
		super(scope, id, props);

		const bucket = new s3.Bucket(this, "Bucket", {
			bucketName: BUCKET_NAME,
			blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
			encryption: s3.BucketEncryption.S3_MANAGED,
			enforceSSL: true,
			versioned: true,
			lifecycleRules: [{ noncurrentVersionExpiration: Duration.days(30) }],
			removalPolicy: RemovalPolicy.RETAIN,
		});

		const user = new iam.User(this, "BackupUser");
		user.addToPolicy(
			new iam.PolicyStatement({
				actions: ["s3:ListBucket"],
				resources: [bucket.bucketArn],
			}),
		);
		user.addToPolicy(
			new iam.PolicyStatement({
				actions: ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
				resources: [bucket.arnForObjects("*")],
			}),
		);

		const key = new iam.AccessKey(this, "BackupAccessKey", { user });
		const credential = new secretsmanager.Secret(this, "BackupCredential", {
			secretName: "cubby/litestream",
			description: "Litestream S3 credential for cubby-backups",
			secretObjectValue: {
				AWS_ACCESS_KEY_ID: SecretValue.unsafePlainText(key.accessKeyId),
				AWS_SECRET_ACCESS_KEY: key.secretAccessKey,
			},
			removalPolicy: RemovalPolicy.RETAIN,
		});

		new CfnOutput(this, "BucketName", { value: bucket.bucketName });
		new CfnOutput(this, "BucketRegion", { value: this.region });
		new CfnOutput(this, "CredentialSecretName", { value: credential.secretName });
	}
}
