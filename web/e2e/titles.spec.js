import { test, expect } from '@playwright/test';

const ROUTES = ['/', '/watch', '/signed-out', '/parent/children', '/parent/allow', '/parent/review', '/parent/history', '/parent/requests'];

for (const route of ROUTES)
	test(`${route} sets a page title`, async ({ page }) => {
		await page.route('**/api/**', (r) => r.fulfill({ json: [] }));
		await page.goto(route);
		await expect.poll(() => page.title()).toMatch(/^(.+ · )?Cubby$/);
		const title = await page.title();
		expect(title).not.toBe(page.url());
		expect(title).not.toContain('localhost');
	});
