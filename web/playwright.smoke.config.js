import { defineConfig } from '@playwright/test';

// The same walk against a deployed instance (deploy/smoke.sh): SMOKE_BASE_URL, and optionally
// SMOKE_DEVICE_TOKEN for the child's steps (the parent steps check what a signed-out browser meets). No stack is started.
export default defineConfig({
	testDir: 'e2e-full',
	testMatch: 'walk.spec.js',
	workers: 1,
	retries: 0,
	reporter: 'list',
	use: { baseURL: process.env.SMOKE_BASE_URL }
});
