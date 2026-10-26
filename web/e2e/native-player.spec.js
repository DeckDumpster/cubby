import { test, expect } from '@playwright/test';

// A fake IFrame API stands in for YouTube: it records the calls the page makes and lets the
// test drive the player's events. The app's message handler is a recorder.
const FAKE_API = `
window.YT = { Player: function (id, opts) {
	var self = this; window.__yt = self; self.opts = opts; self.calls = []; self.t = 12.5;
	self.getCurrentTime = function () { return self.t; };
	self.getDuration = function () { return 300; };
	self.getVideoData = function () { return { video_id: opts.videoId }; };
	self.playVideo = function () { self.calls.push('play'); opts.events.onStateChange({ data: 1 }); };
	self.pauseVideo = function () { self.calls.push('pause'); opts.events.onStateChange({ data: 2 }); };
	self.seekTo = function (t) { self.calls.push('seek:' + t); self.t = t; };
	self.destroy = function () { self.calls.push('destroy'); };
	setTimeout(opts.events.onReady, 0);
} };
setTimeout(function () { window.onYouTubeIframeAPIReady(); }, 0);
`;

test('@native the page bridges the IFrame player to the app', async ({ page }) => {
	const requests = [];
	page.on('request', (r) => requests.push(r.url()));
	await page.route('https://www.youtube.com/iframe_api', (route) =>
		route.fulfill({ contentType: 'text/javascript', body: FAKE_API })
	);
	await page.addInitScript(() => {
		window.__msgs = [];
		window.webkit = { messageHandlers: { cubby: { postMessage: (m) => window.__msgs.push(m) } } };
	});
	const res = await page.goto('/native/player');
	expect(res.headers()['cache-control']).toBe('no-store');
	await page.waitForFunction(() => window.YT && window.cubby);
	await page.evaluate(() => window.cubby.load('abc123', 42.9));
	await expect.poll(() => page.evaluate(() => window.__msgs.map((m) => m.type))).toEqual(['ready']);
	const vars = await page.evaluate(() => window.__yt.opts);
	expect(vars.host).toBe('https://www.youtube-nocookie.com');
	expect(vars.playerVars).toMatchObject({ autoplay: 1, start: 42, controls: 0, fs: 0 });
	expect(vars.playerVars.origin).toBe(new URL(page.url()).origin);

	await page.evaluate(() => window.cubby.play());
	await expect
		.poll(() => page.evaluate(() => window.__msgs.filter((m) => m.type === 'time').length))
		.toBeGreaterThan(0);
	await page.evaluate(() => window.cubby.seek(100));
	await page.evaluate(() => window.cubby.pause());
	await page.evaluate(() => {
		window.__yt.opts.events.onError({ data: 150 });
		window.__yt.opts.events.onAutoplayBlocked();
	});
	await page.evaluate(() => window.cubby.destroy());

	const msgs = await page.evaluate(() => window.__msgs);
	const order = msgs.map((m) => m.type).filter((t, i, a) => t !== 'time' || a[i - 1] !== 'time');
	expect(order).toEqual([
		'ready', 'state', 'video', 'time', 'state', 'video', 'error', 'autoplayBlocked'
	]);
	expect(msgs[1]).toEqual({ type: 'state', state: 1 });
	expect(msgs[2]).toEqual({ type: 'video', id: 'abc123' });
	expect(msgs.find((m) => m.type === 'time')).toEqual({ type: 'time', t: 12.5, d: 300 });
	expect(msgs.find((m) => m.type === 'error')).toEqual({ type: 'error', code: 150 });
	expect(await page.evaluate(() => window.__yt.calls)).toEqual(['play', 'seek:100', 'pause', 'destroy']);
	const before = msgs.length;
	await page.waitForTimeout(700);
	expect(await page.evaluate(() => window.__msgs.length)).toBe(before);
	expect(requests.filter((u) => new URL(u).pathname.startsWith('/api'))).toEqual([]);
});

test('@native without a webkit handler the page posts nothing and does not throw', async ({ page }) => {
	const errors = [];
	page.on('pageerror', (e) => errors.push(e));
	await page.route('https://www.youtube.com/iframe_api', (route) =>
		route.fulfill({ contentType: 'text/javascript', body: FAKE_API })
	);
	await page.goto('/native/player');
	await page.waitForFunction(() => window.YT && window.cubby);
	await page.evaluate(() => window.cubby.load('abc123', 0));
	await page.evaluate(() => window.cubby.play());
	await page.evaluate(() => window.cubby.destroy());
	expect(errors).toEqual([]);
});
