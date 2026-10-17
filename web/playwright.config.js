import { defineConfig } from '@playwright/test';

export default defineConfig({
	testDir: 'e2e',
	fullyParallel: true,
	reporter: 'list',
	use: { baseURL: 'http://localhost:4173' },
	webServer: {
		command: 'npm run dev -- --port 4173 --strictPort',
		url: 'http://localhost:4173/watch',
		reuseExistingServer: !process.env.CI
	}
});
