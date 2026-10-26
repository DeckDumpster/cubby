import test from 'node:test';
import assert from 'node:assert/strict';
import {
	isShell,
	badge,
	refusalMessage,
	localUrl,
	PLAYER_VARS,
	PLAYER_HOST,
	beatDue,
	endReason,
	loadYT,
	canSaveForLater,
	watchLaterPath
} from '../src/lib/child.js';

test('the pause card is for the CubbyShell user agent only', () => {
	assert.equal(isShell('Mozilla/5.0 (iPad) CubbyShell/1.2'), true);
	assert.equal(isShell('Mozilla/5.0 (iPad) Safari/605'), false);
	assert.equal(isShell(undefined), false);
});

test('the badge names videos left and carries a meter fraction', () => {
	assert.deepEqual(badge({ videos: 3, minutes: null }, 5), { text: '3 videos left today', frac: 0.6 });
	assert.equal(badge({ videos: 1, minutes: null }, 5)?.text, '1 video left today');
	assert.equal(badge({ videos: 0, minutes: null }, 5)?.text, 'All done for today');
	assert.equal(badge({ videos: null, minutes: 12.4 }, null)?.text, '12 minutes left today');
	assert.equal(badge({ videos: null, minutes: null }, null), null);
});

test('the badge never contradicts the server when both limits are set', () => {
	const done = 'All done for today';
	assert.equal(badge({ videos: 1, minutes: 0, ve_minutes: 15 }, 3)?.text, done);
	assert.equal(badge({ videos: 0, minutes: 10, ve_minutes: 15 }, 3)?.text, done);
	assert.equal(badge({ videos: 1.5, minutes: 0.3, ve_minutes: 15 }, 3)?.text, '1 minute left today');
	assert.equal(badge({ videos: 0.62, minutes: 9, ve_minutes: 15 }, 3)?.text, '9 minutes left today');
	assert.equal(badge({ videos: 0.62, minutes: null }, 3)?.text, 'A little more time today');
	assert.ok((badge({ videos: 0.62, minutes: null }, 3)?.frac ?? 0) > 0);
	assert.equal(badge({ videos: 2, minutes: 5, ve_minutes: 15 }, 3)?.text, '5 minutes left today');
	assert.equal(badge({ videos: 2, minutes: 60, ve_minutes: 15 }, 3)?.text, '2 videos left today');
	assert.equal(badge({ videos: 1, minutes: 60 }, 3)?.text, '1 video left today');
});

test('refusals are worded for a child, each reason differently', () => {
	const texts = ['out-of-videos', 'out-of-minutes', 'not-now', 'not-allowed'].map((r) => refusalMessage(r, null));
	assert.equal(new Set(texts).size, 4);
	assert.match(refusalMessage('not-now', Date.UTC(2026, 9, 17, 15, 0) / 1000, 'en-US', 'UTC'), /3:00/);
});

test('only same-origin urls survive', () => {
	assert.equal(localUrl('/thumbs/a.jpg', 'https://cubby.gantt.pub'), '/thumbs/a.jpg');
	assert.equal(localUrl('https://cubby.gantt.pub/thumbs/a.jpg', 'https://cubby.gantt.pub'), 'https://cubby.gantt.pub/thumbs/a.jpg');
	assert.equal(localUrl('https://i.ytimg.com/vi/a.jpg', 'https://cubby.gantt.pub'), '');
	assert.equal(localUrl('//evil.example/x', 'https://cubby.gantt.pub'), '');
	assert.equal(localUrl(undefined, 'https://cubby.gantt.pub'), '');
});

test('the player is configured as the LLD says', () => {
	assert.equal(PLAYER_HOST, 'https://www.youtube-nocookie.com');
	assert.deepEqual(PLAYER_VARS, { autoplay: 1, rel: 0, playsinline: 1, fs: 0, iv_load_policy: 3, controls: 0, disablekb: 1 });
});

test('a beat is due every 10 seconds', () => {
	assert.equal(beatDue(1000, 1000), false);
	assert.equal(beatDue(1000, 10999), false);
	assert.equal(beatDue(1000, 11000), true);
});

test('end reasons', () => {
	assert.equal(endReason('ended'), 'ended');
	assert.equal(endReason('stop'), 'stop');
	assert.equal(endReason('anything else'), 'left');
});

test('loadYT resolves at once when the API is already there', async () => {
	const YT = { Player: class {} };
	assert.equal(await loadYT({ YT }), YT);
});

import { readFileSync } from 'node:fs';
import { parseHome, parseRecent, parseSearch } from '../src/lib/child.js';
const fixture = (/** @type {string} */ n) => JSON.parse(readFileSync(new URL(`./fixtures/${n}`, import.meta.url), 'utf8'));

test('the page reads the server home payload: sections carry titled tiles', () => {
	const { sections } = parseHome(fixture('child-home.json'));
	assert.equal(sections[0].title, 'Everything');
	assert.equal(sections[0].tiles[0].youtube_id, 'a');
});

test('the page reads the server search payload as tiles', () => {
	assert.equal(parseSearch(fixture('child-search.json'))[0].youtube_id, 'a');
});

test('parseRecent reads tiles and the more flag, and survives a bad payload', () => {
	assert.deepEqual(parseRecent({ tiles: [{ youtube_id: 'a' }], more: true }), { tiles: [{ youtube_id: 'a' }], more: true });
	assert.deepEqual(parseRecent(null), { tiles: [], more: false });
});

import { graceText } from '../src/lib/child.js';
import { length as lengthText } from '../src/lib/child.js';

test('the grace countdown reads in plain words', () => {
	assert.equal(graceText(120), '2 minutes to look around');
	assert.equal(graceText(61), '2 minutes to look around');
	assert.equal(graceText(60), '60 seconds to look around');
	assert.equal(graceText(1), '1 second to look around');
	assert.equal(graceText(-5), '0 seconds to look around');
});

import { moreAfterThis, startNotice, minutesLeft, timeNotice } from '../src/lib/child.js';

test('notices: counts after this video, nudge stages', () => {
	assert.strictEqual(moreAfterThis(3), 2);
	assert.strictEqual(moreAfterThis(1.5), 0.5);
	assert.strictEqual(moreAfterThis(1), 0);
	assert.strictEqual(startNotice(moreAfterThis(1.64, 7 * 60, 15)), 'You can watch 1 more video after this one.');
	assert.strictEqual(startNotice(moreAfterThis(1.64, 15 * 60, 15)), 'You have a little more time after this one.');
	assert.strictEqual(startNotice(moreAfterThis(1, 15 * 60, 15)), 'This is the last video.');
	assert.strictEqual(
		startNotice(moreAfterThis(0.5, 15 * 60, 15)),
		'This is the last video. Videos stop partway through this one.'
	);
	assert.strictEqual(startNotice(moreAfterThis(1.64, null, 15)), 'You have a little more time after this one.');
	assert.strictEqual(moreAfterThis(null), null);
	assert.strictEqual(startNotice(0), 'This is the last video.');
	assert.strictEqual(startNotice(1), 'You can watch 1 more video after this one.');
	assert.strictEqual(minutesLeft({ minutes: 9, window_left_s: 120 }), 2);
	assert.strictEqual(minutesLeft({ minutes: null }), null);
	assert.strictEqual(timeNotice(6, 0), null);
	assert.deepEqual(timeNotice(4.9, 0), { text: 'After 5 more minutes, videos will stop.', stage: 1 });
	assert.strictEqual(timeNotice(4.5, 1), null);
	assert.deepEqual(timeNotice(0.9, 1), { text: 'After 1 more minute, videos will stop.', stage: 2 });
	assert.strictEqual(timeNotice(0.5, 2), null);
});

test('only a refusal that is not about the video itself offers Save it for later', () => {
	for (const r of ['out-of-videos', 'out-of-minutes', 'not-now']) assert.equal(canSaveForLater(r), true);
	assert.equal(canSaveForLater('not-allowed'), false);
	assert.equal(watchLaterPath('a b'), '/api/child/playlists/watch_later/items/a%20b');
});

const vectors = JSON.parse(readFileSync(new URL('./vectors/budget-text.json', import.meta.url), 'utf8')).vectors;
/** @type {Record<string, (i: any) => any>} */
const fns = {
	badge: (i) => badge(i, null)?.text ?? null,
	moreAfterThis: (i) => moreAfterThis(i.videos, i.duration_s, i.ve_minutes),
	startNotice: (i) => startNotice(i.n),
	timeNotice: (i) => timeNotice(i.minutes, i.stage),
	graceText: (i) => graceText(i.seconds),
	length: (i) => lengthText(i.seconds),
	refusal: (i) => refusalMessage(i.reason, i.next_start, i.locale, i.time_zone)
};

for (const row of vectors) {
	test(`vector ${row.fn} ${JSON.stringify(row.in)}`, () => {
		assert.ok(fns[row.fn], `unknown fn ${row.fn}`);
		assert.deepEqual(fns[row.fn](row.in), row.out);
	});
}
