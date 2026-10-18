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

async function setup(page, { play = { grant_id: 'g1', remaining: { videos: 2, minutes: null } } } = {}) {
	const calls = [];
	await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
	await page.addInitScript(FAKE_PLAYER);
	await page.route('**/api/child/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname;
		calls.push({ path, body: req.postDataJSON?.() ?? null });
		const json = (data) => route.fulfill({ json: data });
		if (path.endsWith('/home')) return json(HOME);
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
	expect(opts.vars).toMatchObject({ autoplay: 0, rel: 0, playsinline: 1, fs: 0, iv_load_policy: 3, controls: 0, disablekb: 1 });
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
