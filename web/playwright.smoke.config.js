import { defineConfig } from '@playwright/test';

// The same walk against a deployed instance (deploy/smoke.sh): SMOKE_BASE_URL, and optionally
// SMOKE_ACCESS_JWT (a test Access identity) for the parent steps and SMOKE_DEVICE_TOKEN for the
// child's. No stack is started.
export default defineConfig({
	testDir: 'e2e-full',
	testMatch: 'walk.spec.js',
	workers: 1,
	retries: 0,
	reporter: 'list',
	use: { baseURL: process.env.SMOKE_BASE_URL }
});
