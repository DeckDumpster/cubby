import { test, expect } from '@playwright/test';

const CHILDREN = [{ id: 'c1', name: 'Lily', avatar: '🦊', tz: 'America/Chicago', devices: [], budget: { video_equivalents: 5, ve_minutes: 15, minutes_cap: 45, cutoff_mid_play: true, daily_windows: [] } }];
const PROGRESS = [{ child_id: 'c1', tz: 'America/Chicago', videos: { limit: 5, used: 1, left: 4, reached: false }, minutes: { limit: 45, used: 10, left: 35, reached: false },
	resets_at: 1, window: { restricted: false, open: true, opens_at: null, closes_at: null }, watching: null, grants: [] }];
const shots = process.env.SHOTS_DIR;

/** Mocks the parent API; POSTs to /extra wait `delay` ms and answer with `status`. */
async function mock(page, { delay = 0, status = 200 } = {}) {
	const extra = [];
	await page.route('**/api/parent/**', async (route) => {
		const r = route.request();
		const u = r.url();
		if (r.method() === 'POST' && u.endsWith('/extra')) {
			extra.push(r.postDataJSON());
			await new Promise((res) => setTimeout(res, delay));
			return route.fulfill({ status, json: status === 200 ? {} : { error: 'the server said no' } });
		}
		const body = u.endsWith('/progress') ? PROGRESS : u.includes('/children') ? CHILDREN : [];
		return route.fulfill({ json: body });
	});
	return extra;
}

for (const scheme of ['light', 'dark'])
	test(`an action shows busy, done and a toast (${scheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme: scheme });
		await page.setViewportSize({ width: 390, height: 844 });
		const extra = await mock(page, { delay: 800 });
		await page.goto('/parent/allow');
		const btn = page.getByRole('button', { name: '+15 min' });
		await btn.click();
		await expect(btn).toBeDisabled();
		await expect(btn).toHaveAttribute('data-state', 'busy');
		await btn.click({ force: true }).catch(() => {});
		if (shots) await page.screenshot({ path: `${shots}/${scheme}-busy.png` });
		await expect(btn).toHaveAttribute('data-state', 'done');
		await expect(btn).toBeEnabled();
		await expect(page.getByRole('region', { name: 'Notifications' })).toContainText('Added 15 min for Lily');
		expect(extra).toHaveLength(1);
		expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
		if (shots) await page.screenshot({ path: `${shots}/${scheme}-done.png` });
		await expect(btn).toHaveAttribute('data-state', 'idle', { timeout: 4000 });
	});

for (const scheme of ['light', 'dark'])
	test(`a failed action re-enables the button and keeps an error toast (${scheme})`, async ({ page }) => {
		await page.emulateMedia({ colorScheme: scheme });
		await page.setViewportSize({ width: 390, height: 844 });
		await mock(page, { status: 500 });
		await page.goto('/parent/allow');
		const btn = page.getByRole('button', { name: '+1 video' });
		await btn.click();
		const region = page.getByRole('region', { name: 'Notifications' });
		await expect(region).toContainText('Could not add the extra time');
		await expect(region).toContainText('the server said no');
		await expect(btn).toBeEnabled();
		await expect(btn).toHaveAttribute('data-state', 'idle');
		await page.waitForTimeout(4500);
		await expect(region).toContainText('Could not add the extra time');
		if (shots) await page.screenshot({ path: `${shots}/${scheme}-failed.png` });
		await page.getByRole('button', { name: 'Dismiss notification' }).click();
		await expect(region).not.toContainText('Could not');
	});
