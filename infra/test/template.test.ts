import assert from "node:assert/strict";
import { test } from "node:test";
import { App } from "aws-cdk-lib";
import { Match, Template } from "aws-cdk-lib/assertions";
import { CubbyBackups } from "../lib/cubby-backups.ts";

const template = Template.fromStack(
	new CubbyBackups(new App(), "CubbyBackups", { env: { account: "111111111111", region: "us-east-1" } }),
);

test("bucket is private, versioned, encrypted, expiring and retained", () => {
	template.resourceCountIs("AWS::S3::Bucket", 1);
	template.hasResource("AWS::S3::Bucket", {
		DeletionPolicy: "Retain",
		UpdateReplacePolicy: "Retain",
		Properties: {
			BucketName: "cubby-backups",
			PublicAccessBlockConfiguration: {
				BlockPublicAcls: true,
				BlockPublicPolicy: true,
				IgnorePublicAcls: true,
				RestrictPublicBuckets: true,
			},
			VersioningConfiguration: { Status: "Enabled" },
			BucketEncryption: {
				ServerSideEncryptionConfiguration: [
					Match.objectLike({ ServerSideEncryptionByDefault: { SSEAlgorithm: "AES256" } }),
				],
			},
			LifecycleConfiguration: {
				Rules: [Match.objectLike({ NoncurrentVersionExpiration: { NoncurrentDays: 30 }, Status: "Enabled" })],
			},
		},
	});
});

test("the user's policy names only this bucket and only object read/write/delete/list", () => {
	const policies = template.findResources("AWS::IAM::Policy");
	const user = Object.values(policies).filter((p) => JSON.stringify(p).includes("BackupUser"));
	assert.equal(user.length, 1);
	const statements = (user[0] as any).Properties.PolicyDocument.Statement as any[];
	const actions = statements.flatMap((s) => [s.Action].flat()).sort();
	assert.deepEqual(actions, ["s3:DeleteObject", "s3:GetObject", "s3:ListBucket", "s3:PutObject"]);
	for (const s of statements) {
		assert.equal(s.Effect, "Allow");
		for (const r of [s.Resource].flat()) {
			const text = JSON.stringify(r);
			assert.ok(text.includes("Bucket"), `resource is not the bucket: ${text}`);
			assert.ok(!text.includes('"*"') || text.includes("Fn::Join"), `bare wildcard resource: ${text}`);
		}
	}
});

test("the only bucket in the template is cubby-backups; no other policy grants anything", () => {
	assert.equal(Object.keys(template.findResources("AWS::IAM::Policy")).length, 1);
	template.resourceCountIs("AWS::IAM::User", 1);
	template.resourceCountIs("AWS::IAM::AccessKey", 1);
});

test("outputs carry bucket name, region and the credential secret", () => {
	template.hasOutput("BucketName", {});
	template.hasOutput("BucketRegion", {});
	template.hasOutput("CredentialSecretName", { Value: Match.anyValue() });
});
