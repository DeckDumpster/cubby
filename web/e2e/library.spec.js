import { test, expect } from '@playwright/test';

// A fixture family: two channels of 400 videos each, grants seeded per video.
const CHANNELS = [['UCaaa', 'Science for Kids'], ['UCbbb', 'Crafty Corner']];
const VIDEOS = CHANNELS.flatMap(([ch, title]) =>
	Array.from({ length: 400 }, (_, i) => ({
		youtube_id: `${ch}-${String(i).padStart(3, '0')}`,
		title: `${title} episode ${String(i).padStart(3, '0')}${i === 7 ? ' volcano special' : ''}`,
		channel_id: ch,
		channel_title: title,
		policy_id: `pol-${ch}`,
		plays: i % 5,
		watched_s: (i % 5) * 90
	}))
);
const shots = process.env.SHOTS_DIR;

async function mock(page) {
	await page.route('**/api/parent/**', (route) => {
		const u = new URL(route.request().url());
		const q = (u.searchParams.get('q') ?? '').toLowerCase();
		const hit = VIDEOS.filter((v) => !q || `${v.title} ${v.channel_title}`.toLowerCase().includes(q));
		if (u.pathname.endsWith('/library/videos')) {
			const g = hit.filter((v) => v.channel_id === u.searchParams.get('group'));
			const sort = u.searchParams.get('sort');
			if (sort === 'plays') g.sort((a, b) => b.plays - a.plays || a.title.localeCompare(b.title));
			const off = Number(u.searchParams.get('offset') ?? 0);
			const lim = Number(u.searchParams.get('limit') ?? 100);
			return route.fulfill({ json: { total: g.length, videos: g.slice(off, off + lim) } });
		}
		if (u.pathname.endsWith('/library')) {
			const groups = CHANNELS.map(([ch, title]) => ({ key: ch, title, count: hit.filter((v) => v.channel_id === ch).length, playlists: [] })).filter((g) => g.count);
			return route.fulfill({ json: { groups } });
		}
		return route.fulfill({ json: u.pathname.endsWith('/children') || u.pathname.endsWith('/policies') || u.pathname.endsWith('/requests') ? [] : {} });
	});
}

for (const path of ['allow', 'review'])
	for (const [w, h] of [[390, 844], [1280, 800]])
		test(`library on /parent/${path} at ${w}px`, async ({ page }) => {
			await page.setViewportSize({ width: w, height: h });
			await mock(page);
			const t0 = Date.now();
			await page.goto(`/parent/${path}`);
			const science = page.getByRole('button', { name: /Science for Kids/ });
			await expect(science).toBeVisible();
			expect(Date.now() - t0, 'first render').toBeLessThan(1000);
			await expect(science).toContainText('400 videos');
			await expect(science).toHaveAttribute('aria-expanded', 'false');
			await expect(page.getByText('episode 000')).toHaveCount(0);

			await science.click();
			await expect(science).toHaveAttribute('aria-expanded', 'true');
			await expect(page.locator('.lib-videos li')).toHaveCount(100);
			// episode 003 was seeded with 3 plays and 270 s watched.
			const row = page.locator('.lib-videos li').filter({ hasText: 'Science for Kids episode 003' });
			await expect(row).toContainText('3 plays');
			await expect(row).toContainText('4m 30s');
			await page.getByRole('button', { name: /Show more/ }).click();
			await expect(page.locator('.lib-videos li')).toHaveCount(200);

			await science.click();
			await expect(science).toHaveAttribute('aria-expanded', 'false');
			expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
			if (shots) await page.screenshot({ path: `${shots}/library-${path}-${w}.png`, fullPage: true });
		});

test('search narrows to a title and keeps the channel heading', async ({ page }) => {
	await mock(page);
	await page.goto('/parent/allow');
	await page.getByLabel('Search videos').fill('volcano');
	const science = page.getByRole('button', { name: /Science for Kids/ });
	await expect(science).toContainText('1 video');
	await expect(page.getByRole('button', { name: /Crafty Corner/ })).toHaveCount(1);
	await expect(page.locator('.lib-videos li')).toHaveCount(2);
	await expect(page.locator('.lib-videos li').first()).toContainText('volcano special');
	await page.getByLabel('Search videos').fill('zzzz');
	await expect(page.getByText('No videos match that search.')).toBeVisible();
});

test('sorting by plays puts the most played first', async ({ page }) => {
	await mock(page);
	await page.goto('/parent/review');
	await page.getByLabel('Sort videos').selectOption('plays');
	await page.getByRole('button', { name: /Crafty Corner/ }).click();
	await expect(page.locator('.lib-videos li').first()).toContainText('4 plays');
});
