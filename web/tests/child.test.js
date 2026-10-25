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
	loadYT
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
import { parseHome, parseSearch } from '../src/lib/child.js';
const fixture = (/** @type {string} */ n) => JSON.parse(readFileSync(new URL(`./fixtures/${n}`, import.meta.url), 'utf8'));

test('the page reads the server home payload: sections carry titled tiles', () => {
	const { sections } = parseHome(fixture('child-home.json'));
	assert.equal(sections[0].title, 'New');
	assert.equal(sections[0].tiles[0].youtube_id, 'a');
});

test('the page reads the server search payload as tiles', () => {
	assert.equal(parseSearch(fixture('child-search.json'))[0].youtube_id, 'a');
});

import { moreAfterThis, startNotice, minutesLeft, timeNotice } from '../src/lib/child.js';

test('notices: counts after this video, nudge stages', () => {
	assert.strictEqual(moreAfterThis(3), 2);
	assert.strictEqual(moreAfterThis(1.5), 0);
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
