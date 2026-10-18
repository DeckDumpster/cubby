import { startStack } from './stack.js';

/** Playwright globalSetup: brings the stack up once; a parent signs in through its fake provider. */
export default async function setup() {
	const stack = await startStack({ ports: { server: Number(process.env.E2E_PORT ?? 4180) } });
	return () => stack.stop();
}
