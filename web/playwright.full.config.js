import { defineConfig } from '@playwright/test';

// The whole-product walk: `npm run e2e:full`. Needs `npm run build` and a built cubby-server
// (CUBBY_SERVER_BIN, default ../target/debug/cubby-server). One worker: the walk is one story.
// E2E_CHROMIUM points at a browser binary when Playwright's own is not installed.
export default defineConfig({
	testDir: 'e2e-full',
	testMatch: 'walk.spec.js',
	globalSetup: './e2e-full/setup.js',
	workers: 1,
	retries: 0,
	reporter: 'list',
	use: {
		baseURL: `http://127.0.0.1:${process.env.E2E_PORT ?? 4180}`,
		...(process.env.E2E_CHROMIUM ? { launchOptions: { executablePath: process.env.E2E_CHROMIUM } } : {})
	}
});
