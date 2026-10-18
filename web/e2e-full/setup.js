import { startStack } from './stack.js';

/** Playwright globalSetup: brings the stack up once and hands the parent's Access JWT to the specs. */
export default async function setup() {
	const stack = await startStack({ ports: { server: Number(process.env.E2E_PORT ?? 4180) } });
	process.env.E2E_PARENT_JWT = stack.parentJwt();
	return () => stack.stop();
}
