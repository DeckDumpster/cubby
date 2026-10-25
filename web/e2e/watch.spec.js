import { test, expect } from '@playwright/test';

const tile = (id, title) => ({ youtube_id: id, title, channel_title: 'Chan', thumb: `/thumbs/${id}.jpg` });
const HOME = {
	child: { name: 'Kid', avatar: '🦄' },
	remaining: { videos: 3, minutes: null },
	total_videos: 5,
	sections: [{ key: 'all', title: 'All videos', tiles: [tile('v1', 'First video'), tile('v2', 'Second video')] }]
};
const NEXT = { tiles: [tile('v2', 'Second video'), tile('v3', 'Third video')], remaining: { videos: 2, minutes: null } };

/** A stand-in for the YouTube IFrame API; the test drives its state by hand. */
const FAKE_PLAYER = () => {
	window.__players = [];
	window.YT = {
		PlayerState: { ENDED: 0, PLAYING: 1, PAUSED: 2 },
		Player: class {
			constructor(el, opts) {
				this.opts = opts;
				this.el = el;
				this.destroyed = false;
				el.innerHTML = '<iframe title="fake player"></iframe>';
				window.__players.push(this);
			}
			getCurrentTime() {
				return this.t ?? 12;
			}
			getDuration() {
				return 100;
			}
			getVideoData() {
				return { video_id: this.vid ?? this.opts.videoId };
			}
			playVideo() {
				if (window.__refuse) return;
				this.played = (this.played ?? 0) + 1;
			}
			pauseVideo() {
				this.paused = true;
			}
			seekTo(t) {
				this.t = t;
			}
			destroy() {
				this.destroyed = true;
				this.el.innerHTML = '';
			}
		}
	};
	window.__state = (s) => window.__players.at(-1).opts.events.onStateChange({ data: s });
};

async function setup(page, { play = { grant_id: 'g1', remaining: { videos: 2, minutes: null } }, recent = [] } = {}) {
	const calls = [];
	await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
	await page.addInitScript(FAKE_PLAYER);
	await page.route('**/api/child/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname;
		calls.push({ path, body: req.postDataJSON?.() ?? null });
		const json = (data) => route.fulfill({ json: data });
		if (path.includes('/playlists/')) return json({ ok: true });
		if (path.endsWith('/home')) return json(HOME);
		if (path.endsWith('/recent')) {
			const q = new URL(req.url()).searchParams;
			const limit = Number(q.get('limit') ?? 24);
			const off = Number(q.get('offset') ?? 0);
			return json({ tiles: recent.slice(off, off + limit), more: recent.length > off + limit });
		}
		if (path.endsWith('/play')) return json(play);
		if (path.endsWith('/beat')) return json({ remaining: { videos: 2, minutes: null }, stop: false });
		if (path.endsWith('/end')) return json({ ok: true });
		if (path.endsWith('/next')) return json(NEXT);
		if (path.endsWith('/search')) return json({ tiles: [tile('v9', 'Found it')] });
		if (path.endsWith('/request')) return json({ ok: true });
		return route.fulfill({ status: 404, json: {} });
	});
	return calls;
}

const offOrigin = (page) =>
	page.evaluate(() =>
		[...document.querySelectorAll('a[href], area[href], form[action]')]
			.map((e) => e.getAttribute('href') ?? e.getAttribute('action'))
			.filter((u) => new URL(u, location.href).origin !== location.origin)
	);

test('the grid shows the budget badge and has no off-origin link anywhere in the flow', async ({ page }) => {
	await setup(page);
	await page.goto('/watch');
	await expect(page.getByTestId('greeting')).toContainText('🦄');
	await expect(page.getByTestId('badge')).toContainText('3 videos left today');
	await expect(page.getByRole('button', { name: /First video/ })).toBeVisible();
	expect(await offOrigin(page)).toEqual([]);

	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	expect(await offOrigin(page)).toEqual([]);

	await page.evaluate(() => window.__state(0));
	await expect(page.getByTestId('end-card')).toBeVisible();
	expect(await offOrigin(page)).toEqual([]);
});

test('the player is built only after a grant, with the privacy host and the LLD options', async ({ page }) => {
	await setup(page);
	await page.goto('/watch');
	expect(await page.evaluate(() => window.__players.length)).toBe(0);
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	const opts = await page.evaluate(() => {
		const o = window.__players[0].opts;
		return { host: o.host, videoId: o.videoId, vars: o.playerVars };
	});
	expect(opts.host).toBe('https://www.youtube-nocookie.com');
	expect(opts.videoId).toBe('v1');
	expect(opts.vars).toMatchObject({ autoplay: 1, rel: 0, playsinline: 1, fs: 0, iv_load_policy: 3, controls: 0, disablekb: 1 });
});

test('ENDED destroys the player and shows the end card with suggestions and the remaining budget', async ({ page }) => {
	const calls = await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	await page.evaluate(() => window.__state(1));
	await page.evaluate(() => window.__state(0));
	const card = page.getByTestId('end-card');
	await expect(card).toBeVisible();
	await expect(page.getByTestId('end-budget')).toHaveText('2 videos left today');
	await expect(card.getByRole('button', { name: /Third video/ })).toBeVisible();
	expect(await page.evaluate(() => window.__players[0].destroyed)).toBe(true);
	expect(calls.find((c) => c.path.endsWith('/end')).body).toEqual({ reason: 'ended' });
	expect(calls.some((c) => c.path.endsWith('/next'))).toBe(true);
});

test('a spent budget shows the refusal card and never builds a player', async ({ page }) => {
	await setup(page, { play: { refused: 'out-of-videos', next_start: null } });
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.getByTestId('refusal')).toContainText('all your videos for today');
	expect(await page.evaluate(() => window.__players.length)).toBe(0);
});

test('like and more-from-channel post requests', async ({ page }) => {
	const calls = await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await page.getByRole('button', { name: /I like this/ }).click();
	await page.getByRole('button', { name: /More from this channel/ }).click();
	const reqs = calls.filter((c) => c.path.endsWith('/request')).map((c) => c.body);
	expect(reqs).toEqual([
		{ kind: 'like', youtube_id: 'v1' },
		{ kind: 'more-from-channel', youtube_id: 'v1' }
	]);
});

test('search shows only what the search API returns', async ({ page }) => {
	await setup(page);
	await page.goto('/watch');
	await page.getByLabel('search').fill('cats');
	await page.getByRole('button', { name: 'Search' }).click();
	await expect(page.getByRole('button', { name: /Found it/ })).toBeVisible();
	await expect(page.getByRole('button', { name: /First video/ })).toHaveCount(0);
});

test('PAUSED hides the iframe and shows our Resume screen; Resume plays on', async ({ page }) => {
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	await page.evaluate(() => window.__state(1));
	await page.evaluate(() => {
		window.__players[0].t = 33;
		window.__state(2);
	});
	await expect(page.getByTestId('pause-card')).toBeVisible();
	await expect(page.locator('iframe[title="fake player"]')).toBeHidden();
	await page.screenshot({ path: 'test-results/paused.png' });
	await page.getByTestId('resume').click();
	await expect(page.getByTestId('pause-card')).toHaveCount(0);
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	const p = await page.evaluate(() => ({ t: window.__players[0].getCurrentTime(), played: window.__players[0].played }));
	expect(p).toEqual({ t: 33, played: 1 });
});

test('a granted player shows no YouTube chrome and Cubby controls drive it', async ({ page }) => {
	await page.setViewportSize({ width: 1024, height: 768 });
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await page.evaluate(() => window.__state(1));
	await expect(page.getByTestId('playpause')).toHaveAccessibleName('Pause');
	await expect(page.getByTestId('clock')).toContainText('0:12 / -1:28');
	await page.screenshot({ path: 'test-results/playing.png' });
	await page.getByTestId('playpause').click();
	expect(await page.evaluate(() => window.__players[0].paused)).toBe(true);
	await page.locator('input[aria-label="seek"]').fill('60');
	expect(await page.evaluate(() => window.__players[0].t)).toBe(60);
	const o = await page.evaluate(() => window.__players[0].opts.playerVars);
	expect(o.controls).toBe(0);
});

test('the iframe stays visibility:hidden behind a ready frame after start and seek until PLAYING has lasted N s', async ({ page }) => {
	await page.setViewportSize({ width: 1024, height: 768 });
	await page.clock.install();
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	const frame = page.locator('iframe[title="fake player"]');
	const hidden = () => frame.evaluate((e) => getComputedStyle(e).visibility);
	await page.evaluate(() => window.__state(1));
	expect(await hidden()).toBe('hidden');
	await expect(page.getByTestId('ready-frame')).toBeVisible();
	await page.screenshot({ path: 'test-results/ready-after-start.png' });
	await page.clock.runFor(3500);
	expect(await hidden()).toBe('hidden');
	await page.clock.runFor(1000);
	expect(await hidden()).toBe('visible');
	await expect(page.getByTestId('ready-frame')).toHaveCount(0);
	await page.screenshot({ path: 'test-results/revealed.png' });
	await page.locator('input[aria-label="seek"]').fill('60');
	expect(await hidden()).toBe('hidden');
	await expect(page.getByTestId('ready-frame')).toBeVisible();
	await expect(page.getByTestId('playpause')).toBeEnabled();
	await page.screenshot({ path: 'test-results/ready-after-seek.png' });
	await page.clock.runFor(4100);
	expect(await hidden()).toBe('visible');
	await page.evaluate(() => window.__state(2));
	await page.getByTestId('resume').click();
	expect(await hidden()).toBe('hidden');
	await page.evaluate(() => window.__state(1));
	await page.clock.runFor(4100);
	expect(await hidden()).toBe('visible');
});

test('an embed that navigates off the granted id is destroyed and the grant ends off-list', async ({ page }) => {
	const calls = await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await page.evaluate(() => window.__state(1));
	await page.evaluate(() => {
		window.__players[0].vid = 'evil';
		window.__state(3);
	});
	await expect(page.getByTestId('stopped')).toBeVisible();
	expect(await page.evaluate(() => window.__players[0].destroyed)).toBe(true);
	expect(calls.find((c) => c.path.endsWith('/end')).body).toEqual({ reason: 'off-list' });
});

test.describe('leaving a search', () => {
	async function searched(page) {
		await setup(page);
		await page.goto('/watch');
		await page.getByLabel('search').fill('golden');
		await page.getByRole('button', { name: 'Search' }).click();
		await expect(page.getByRole('button', { name: /Found it/ })).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Videos about golden' })).toBeVisible();
		await expect(page).toHaveURL(/\/watch\?q=golden/);
	}
	async function atHome(page) {
		await expect(page.getByRole('button', { name: /First video/ })).toBeVisible();
		await expect(page.getByRole('button', { name: /Found it/ })).toHaveCount(0);
		await expect(page.getByLabel('search')).toHaveValue('');
		await expect(page).toHaveURL(/\/watch$/);
	}

	test('the clear button returns home', async ({ page }) => {
		await searched(page);
		await page.getByRole('button', { name: 'Clear' }).click();
		await atHome(page);
	});
	test('the Home control returns home', async ({ page }) => {
		await searched(page);
		await page.getByRole('button', { name: /Home/ }).click();
		await atHome(page);
	});
	test('emptying the box returns home', async ({ page }) => {
		await searched(page);
		await page.getByLabel('search').fill('');
		await atHome(page);
	});
	test('browser back returns home', async ({ page }) => {
		await searched(page);
		await page.goBack();
		await atHome(page);
	});
	test('reloading a search url shows the results', async ({ page }) => {
		await setup(page);
		await page.goto('/watch?q=golden');
		await expect(page.getByRole('button', { name: /Found it/ })).toBeVisible();
		await expect(page.getByLabel('search')).toHaveValue('golden');
	});
});

const startPlaying = async (page) => {
	await page.setViewportSize({ width: 1024, height: 768 });
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	await page.evaluate(() => window.__state(1));
};

test('the fullscreen toggle fullscreens Cubby\'s container, not the iframe, and follows system exit', async ({ page }) => {
	await startPlaying(page);
	const fs = page.getByTestId('fullscreen');
	await expect(fs).toHaveAccessibleName('Full screen');
	await fs.click();
	await expect(fs).toHaveAccessibleName('Exit full screen');
	expect(await page.evaluate(() => document.fullscreenElement?.getAttribute('data-testid'))).toBe('fsbox');
	expect(await page.evaluate(() => !!document.fullscreenElement.querySelector('iframe') && document.fullscreenElement.tagName)).toBe('DIV');
	await page.evaluate(() => document.exitFullscreen());
	await expect(fs).toHaveAccessibleName('Full screen');
	await expect(page.getByTestId('fsbox')).toHaveAttribute('data-fs', 'off');
});

test('with element fullscreen unavailable the toggle fills the viewport with CSS', async ({ page }) => {
	await page.addInitScript(() => {
		Object.defineProperty(document, 'fullscreenEnabled', { value: false });
		Object.defineProperty(document, 'webkitFullscreenEnabled', { value: false });
	});
	await startPlaying(page);
	await page.getByTestId('fullscreen').click();
	const box = page.getByTestId('fsbox');
	await expect(box).toHaveAttribute('data-fs', 'fill');
	expect(await page.evaluate(() => document.fullscreenElement)).toBe(null);
	const r = await box.boundingBox();
	expect(r).toMatchObject({ x: 0, y: 0, width: 1024, height: 768 });
	await page.getByTestId('fullscreen').click();
	await expect(box).toHaveAttribute('data-fs', 'off');
});

test('pausing in fullscreen shows the Resume screen inside the container and hides the iframe', async ({ page }) => {
	await startPlaying(page);
	await page.getByTestId('fullscreen').click();
	await page.evaluate(() => window.__state(2));
	await expect(page.getByTestId('fsbox').getByTestId('pause-card')).toBeVisible();
	await expect(page.locator('iframe[title="fake player"]')).toBeHidden();
});

test('the end card renders inside the fullscreen container', async ({ page }) => {
	await startPlaying(page);
	await page.getByTestId('fullscreen').click();
	await page.evaluate(() => window.__state(0));
	await expect(page.getByTestId('fsbox').getByTestId('end-card')).toBeVisible();
	expect(await page.evaluate(() => document.fullscreenElement?.getAttribute('data-testid'))).toBe('fsbox');
});

test('opening a video from a tile starts playback with no Play press', async ({ page }) => {
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	await page.evaluate(() => window.__players[0].opts.events.onReady({}));
	expect(await page.evaluate(() => window.__players[0].played)).toBe(1);
	await page.evaluate(() => window.__state(1));
	await expect(page.getByTestId('playpause')).toHaveAccessibleName('Pause');
});

test('autoplay refused clears the ready frame and leaves the Play control', async ({ page }) => {
	await page.clock.install();
	await page.addInitScript(() => (window.__refuse = true));
	await setup(page);
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).click();
	await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
	await page.evaluate(() => window.__players[0].opts.events.onReady({}));
	await expect(page.getByTestId('ready-frame')).toBeVisible();
	await page.clock.runFor(3100);
	await expect(page.getByTestId('ready-frame')).toHaveCount(0);
	await expect(page.getByTestId('playpause')).toHaveAccessibleName('Play');
});

test('a shield covers the iframe exactly, takes every tap and toggles playback; the iframe gets no pointer events', async ({ page }) => {
	await page.setViewportSize({ width: 1024, height: 768 });
	await startPlaying(page);
	const same = async () => {
		await expect(page.locator('.player')).toHaveAttribute('data-settled', 'true', { timeout: 8000 });
		const f = await page.locator('.player').boundingBox();
		const s = await page.getByTestId('shield').boundingBox();
		expect(s).toEqual(f);
		expect(await page.evaluate(() => {
			const r = document.querySelector('iframe').getBoundingClientRect();
			return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2).getAttribute('data-testid');
		})).toBe('shield');
		expect(await page.evaluate(() => getComputedStyle(document.querySelector('iframe')).pointerEvents)).toBe('none');
	};
	await same();
	await page.getByTestId('shield').click();
	expect(await page.evaluate(() => window.__players[0].paused)).toBe(true);
	await page.evaluate(() => window.__state(2));
	await page.getByTestId('resume').click();
	await page.evaluate(() => window.__state(1));
	await page.getByTestId('fullscreen').click();
	await same();
	await page.getByTestId('shield').click();
	expect(await page.evaluate(() => window.__players[0].paused)).toBe(true);
});

const SIZES = [
	['ipad-landscape', 1180, 820],
	['ipad-portrait', 820, 1180],
	['phone', 390, 844]
];
for (const [name, width, height] of SIZES) {
	test(`${name}: the video starts smaller so every control stays in view, fullscreen still fills`, async ({ page }) => {
		await page.setViewportSize({ width, height });
		await setup(page);
		await page.goto('/watch');
		await page.getByRole('button', { name: /First video/ }).click();
		await expect(page.locator('iframe[title="fake player"]')).toBeAttached();
		await page.evaluate(() => window.__state(1));
		await expect(page.locator('.player')).toHaveAttribute('data-settled', 'true', { timeout: 8000 });
		for (const el of [
			page.getByTestId('playpause'),
			page.getByRole('slider', { name: 'seek' }),
			page.getByTestId('fullscreen'),
			page.getByTestId('clock'),
			page.getByTestId('badge')
		]) {
			const r = await el.boundingBox();
			expect(r.x).toBeGreaterThanOrEqual(0);
			expect(r.y).toBeGreaterThanOrEqual(0);
			expect(r.x + r.width).toBeLessThanOrEqual(width);
			expect(r.y + r.height).toBeLessThanOrEqual(height);
		}
		expect(await page.evaluate(() => scrollY)).toBe(0);
		const s = await page.getByTestId('shield').boundingBox();
		expect(s).toEqual(await page.locator('.player').boundingBox());
		if (process.env.SHOTS_DIR) await page.screenshot({ path: `${process.env.SHOTS_DIR}/watch-${name}.png` });

		await page.evaluate(() => {
			Object.defineProperty(document, 'fullscreenEnabled', { value: false });
			Object.defineProperty(document, 'webkitFullscreenEnabled', { value: false });
		});
		await page.getByTestId('fullscreen').click();
		await expect(page.getByTestId('fsbox')).toHaveAttribute('data-fs', 'fill');
		const st = await page.locator('.stage').boundingBox();
		expect(st.width).toBeGreaterThan(width - 40);
	});
}

const HISTORY = Array.from({ length: 30 }, (_, i) => tile(`h${i}`, `Watched ${i}`));

for (const [name, size] of [['ipad', { width: 820, height: 1180 }], ['phone', { width: 390, height: 844 }]]) {
	test(`recently watched: one row with See all, then the full list (${name})`, async ({ page }) => {
		await page.setViewportSize(size);
		await setup(page, { recent: HISTORY });
		await page.goto('/watch');
		const row = page.getByTestId('recent-row');
		await expect(row).toBeVisible();
		const tops = await row.locator('.tile').evaluateAll((els) => {
			const rail = els[0].parentElement.getBoundingClientRect();
			return els.filter((e) => e.getBoundingClientRect().top < rail.bottom - 1).map((e) => Math.round(e.getBoundingClientRect().top));
		});
		expect(tops.length).toBeGreaterThan(0);
		expect(new Set(tops).size).toBe(1);
		await page.screenshot({ path: `../docs/screenshots/recent/row-${name}.png` });

		await row.getByRole('button', { name: 'See all' }).click();
		await expect(page).toHaveURL(/recent=1/);
		await expect(page.getByTestId('recent-all').locator('.tile')).toHaveCount(24);
		await page.getByRole('button', { name: 'More' }).click();
		await expect(page.getByTestId('recent-all').locator('.tile')).toHaveCount(30);
		await page.screenshot({ path: `../docs/screenshots/recent/all-${name}.png` });

		await page.getByRole('button', { name: /Home/ }).click();
		await expect(page.getByTestId('recent-row')).toBeVisible();
	});
}

test('every tile has a Watch later control that saves and unsaves, and the row shows the saved videos', async ({ page }) => {
	const calls = await setup(page);
	await page.route('**/api/child/home', (route) =>
		route.fulfill({ json: { ...HOME, saved: ['v2'], watch_later: [tile('v2', 'Second video')] } })
	);
	await page.goto('/watch');
	const row = page.getByTestId('watch-later-row');
	await expect(row).toContainText('Second video');
	const first = page.locator('.tilewrap', { hasText: 'First video' }).getByTestId('save-later');
	await expect(first).toContainText('Later');
	await first.click();
	await expect(first).toContainText('Saved');
	expect(calls.some((c) => c.path.endsWith('/playlists/watch_later/items/v1'))).toBe(true);
	await page.screenshot({ path: test.info().outputPath('landing.png'), fullPage: true });
	await first.click();
	await expect(first).toContainText('Later');
});

test('a refusal offers Save it for later in one tap', async ({ page }) => {
	const calls = await setup(page, { play: { refused: 'out-of-minutes', next_start: null } });
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).first().click();
	await page.getByTestId('save-for-later').click();
	await expect(page.getByTestId('saved-note')).toBeVisible();
	expect(calls.some((c) => c.path.endsWith('/playlists/watch_later/items/v1'))).toBe(true);
	await page.screenshot({ path: test.info().outputPath('refusal.png') });
});

test('a video that is not allowed cannot be saved from the refusal card', async ({ page }) => {
	await setup(page, { play: { refused: 'not-allowed', next_start: null } });
	await page.goto('/watch');
	await page.getByRole('button', { name: /First video/ }).first().click();
	await expect(page.getByTestId('refusal')).toBeVisible();
	await expect(page.getByTestId('save-for-later')).toHaveCount(0);
});
