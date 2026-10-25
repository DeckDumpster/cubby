import { test, expect } from '@playwright/test';

const tile = (id, title) => ({ youtube_id: id, title, channel_title: 'Chan', thumb: `/thumbs/${id}.jpg` });
const home = (limit) => ({
	child: { name: 'Kid', avatar: '🦄' },
	remaining: { videos: 0, minutes: null },
	total_videos: 5,
	limit,
	sections: [{ key: 'all', title: 'All videos', tiles: [tile('v1', 'First video')] }]
});

async function setup(page, status, body) {
	await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
	await page.route('**/api/child/home', (route) => route.fulfill({ status, json: body }));
	await page.route('**/thumbs/**', (route) => route.fulfill({ status: 404 }));
}

for (const [name, size] of [
	['ipad', { width: 820, height: 1180 }],
	['phone', { width: 390, height: 844 }]
]) {
	test(`${name}: the grace shows a countdown and browsing stays`, async ({ page }) => {
		await page.setViewportSize(size);
		await setup(page, 200, home({ state: 'grace', grace_left_s: 120, grace_ends_at: 0 }));
		await page.goto('/watch');
		await expect(page.getByTestId('grace')).toHaveText('2 minutes to look around');
		await expect(page.getByText('First video')).toBeVisible();
		await page.screenshot({ path: `../docs/screenshots/limit/grace-${name}.png` });
	});

	test(`${name}: once locked the page shows only All done for today`, async ({ page }) => {
		await page.setViewportSize(size);
		await setup(page, 403, { error: 'locked', limit: { state: 'locked' } });
		await page.goto('/watch');
		await expect(page.getByTestId('locked')).toContainText('All done for today');
		await expect(page.getByText('First video')).toHaveCount(0);
		await page.screenshot({ path: `../docs/screenshots/limit/locked-${name}.png` });
	});
}
