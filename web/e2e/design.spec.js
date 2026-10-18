import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mkdirSync } from 'node:fs';

// Every /watch state, at the three sizes the child uses: axe (incl. WCAG AA contrast) must pass and a
// screenshot is written to SHOTS_DIR (default screenshots/) for the close note.
const SIZES = [
	['1024x768', { width: 1024, height: 768 }],
	['768x1024', { width: 768, height: 1024 }],
	['1280x800', { width: 1280, height: 800 }]
];
const OUT = process.env.SHOTS_DIR ?? 'screenshots';
mkdirSync(OUT, { recursive: true });

const tile = (id, title, ch = 'Chan') => ({ youtube_id: id, title, channel_title: ch, thumb: `/thumbs/${id}.jpg` });
const MANY = Array.from({ length: 6 }, (_, i) => tile(`v${i}`, `Video number ${i + 1} with a rather long title to wrap`, `Channel ${i}`));
const HOME = {
	child: { name: 'Kid' },
	remaining: { videos: 3, minutes: null },
	total_videos: 5,
	sections: [
		{ key: 'new', title: 'New', tiles: MANY.slice(0, 3) },
		{ key: 'all', title: 'All videos', tiles: MANY }
	]
};
const EMPTY = { child: { name: 'Kid' }, remaining: { videos: 3, minutes: null }, total_videos: 0, sections: [{ key: 'all', title: 'All videos', tiles: [] }] };
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

const FAKE_PLAYER = () => {
	window.__players = [];
	window.YT = {
		PlayerState: { ENDED: 0, PLAYING: 1, PAUSED: 2 },
		Player: class {
			constructor(el, opts) {
				this.opts = opts;
				el.innerHTML = '<iframe title="fake player" style="background:#222"></iframe>';
				window.__players.push(this);
			}
			getCurrentTime() { return 12; }
			destroy() {}
		}
	};
	window.__state = (s) => window.__players.at(-1).opts.events.onStateChange({ data: s });
};

async function setup(page, { home = HOME, play = { grant_id: 'g1', remaining: { videos: 2, minutes: null } }, paired = true, pairStatus = 200 } = {}) {
	if (paired) await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
	await page.addInitScript(FAKE_PLAYER);
	await page.route('**/thumbs/**', (r) => r.fulfill({ body: PNG, contentType: 'image/png' }));
	await page.route('**/api/child/**', (route) => {
		const path = new URL(route.request().url()).pathname;
		const json = (data, status = 200) => route.fulfill({ json: data, status });
		if (path.endsWith('/home')) return json(home);
		if (path.endsWith('/pair')) return json({}, pairStatus);
		if (path.endsWith('/play')) return json(play);
		if (path.endsWith('/beat')) return json({ remaining: { videos: 2, minutes: null }, stop: false });
		if (path.endsWith('/end')) return json({ ok: true });
		if (path.endsWith('/next')) return json({ tiles: MANY.slice(1, 4), remaining: { videos: 2, minutes: null } });
		if (path.endsWith('/search')) return json({ tiles: MANY.slice(0, 4) });
		return json({}, 404);
	});
}

async function check(page, name, size) {
	await page.waitForTimeout(150);
	await page.screenshot({ path: `${OUT}/${name}-${size}.png`, fullPage: true });
	const run = () => new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
	// axe can lose its frame under a loaded runner; one retry, never a skip.
	const r = await run().catch(run);
	expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`)).toEqual([]);
	const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
	expect(overflow).toBeLessThanOrEqual(0);
}

const first = (page) => page.getByRole('button', { name: /Video number 1 / }).first();

for (const [size, viewport] of SIZES) {
	test.describe(size, () => {
		test.use({ viewport });

		test('pairing', async ({ page }) => {
			await setup(page, { paired: false });
			await page.goto('/watch');
			const input = page.getByLabel('pairing code');
			await input.fill('ab3d9z');
			await expect(input).toHaveValue('AB3D9Z');
			const box = await input.boundingBox();
			expect(box.height).toBeGreaterThanOrEqual(64);
			const go = await page.getByRole('button', { name: 'Go' }).boundingBox();
			expect(go.height).toBeLessThan(120);
			expect(go.width).toBeLessThan(600);
			await check(page, 'pairing', size);
		});

		test('pairing error', async ({ page }) => {
			await setup(page, { paired: false, pairStatus: 400 });
			await page.goto('/watch');
			await page.getByLabel('pairing code').fill('AAAAAA');
			await page.getByRole('button', { name: 'Go' }).click();
			await expect(page.getByRole('alert')).toBeVisible();
			await check(page, 'pairing-error', size);
		});

		test('home', async ({ page }) => {
			await setup(page);
			await page.goto('/watch');
			await expect(first(page)).toBeVisible();
			await check(page, 'home', size);
		});

		test('empty', async ({ page }) => {
			await setup(page, { home: EMPTY });
			await page.goto('/watch');
			await expect(page.getByTestId('nothing-allowed')).toBeVisible();
			await check(page, 'empty', size);
		});

		test('search', async ({ page }) => {
			await setup(page);
			await page.goto('/watch');
			await page.getByLabel('search').fill('cats');
			await page.getByRole('button', { name: 'Search' }).click();
			await expect(page.getByRole('heading', { name: /Videos about cats/ })).toBeVisible();
			await check(page, 'search', size);
		});

		test('player', async ({ page }) => {
			await setup(page);
			await page.goto('/watch');
			await first(page).click();
			await expect(page.locator('iframe[title="fake player"]')).toBeVisible();
			await check(page, 'player', size);
		});

		test('end card', async ({ page }) => {
			await setup(page);
			await page.goto('/watch');
			await first(page).click();
			await expect(page.locator('iframe[title="fake player"]')).toBeVisible();
			await page.evaluate(() => window.__state(0));
			await expect(page.getByTestId('end-card')).toBeVisible();
			await check(page, 'end', size);
		});

		test('refused', async ({ page }) => {
			await setup(page, { play: { refused: 'out-of-videos', next_start: null } });
			await page.goto('/watch');
			await first(page).click();
			await expect(page.getByTestId('refusal')).toBeVisible();
			await check(page, 'refused', size);
		});

		test('stopped', async ({ page }) => {
			await setup(page);
			let beats = 0;
			await page.route('**/api/child/grant/*/beat', (r) => r.fulfill({ json: { remaining: { videos: 0, minutes: null }, stop: beats++ === 0 } }));
			await page.goto('/watch');
			await first(page).click();
			await expect(page.locator('iframe[title="fake player"]')).toBeVisible();
			await page.evaluate(() => window.__state(2));
			await expect(page.getByTestId('stopped')).toBeVisible();
			await check(page, 'stopped', size);
		});
	});
}

test.describe('pause card in the shell', () => {
	test.use({ userAgent: 'Mozilla/5.0 (iPad; CPU OS 17_0) CubbyShell/1.0', viewport: SIZES[0][1] });
	test('pause', async ({ page }) => {
		await setup(page);
		await page.goto('/watch');
		await first(page).click();
		await expect(page.locator('iframe[title="fake player"]')).toBeVisible();
		await page.evaluate(() => window.__state(1));
		await page.evaluate(() => window.__state(2));
		await expect(page.getByTestId('pause-card')).toBeVisible();
		await check(page, 'pause', '1024x768');
	});
});

test('the grown-up unpair control returns to the pairing screen', async ({ page }) => {
	await setup(page);
	page.on('dialog', (d) => d.accept());
	await page.goto('/watch');
	await page.getByRole('button', { name: /unpair/i }).click();
	await expect(page.getByLabel('pairing code')).toBeVisible();
	expect(await page.evaluate(() => localStorage.getItem('cubby_token'))).toBeNull();
});
