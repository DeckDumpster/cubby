import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
	testDir: 'e2e',
	fullyParallel: true,
	reporter: 'list',
	use: { baseURL: 'http://localhost:4173' },
	projects: [
		{ name: 'chromium', use: { ...devices['Desktop Chrome'] } },
		// The child's device is an iPad: the autoplay specs also run on WebKit at its viewport.
		{ name: 'webkit-ipad', use: { ...devices['iPad (gen 7)'] }, grep: /@autoplay|@native/ }
	],
	webServer: {
		command: 'npm run dev -- --port 4173 --strictPort',
		url: 'http://localhost:4173/watch',
		reuseExistingServer: !process.env.CI
	}
});
