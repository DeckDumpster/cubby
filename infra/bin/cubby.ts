import { App } from "aws-cdk-lib";
import { CubbyBackups } from "../lib/cubby-backups.ts";

const app = new App();
new CubbyBackups(app, "CubbyBackups", {
	env: { account: process.env.CDK_DEFAULT_ACCOUNT, region: process.env.CDK_DEFAULT_REGION },
});
