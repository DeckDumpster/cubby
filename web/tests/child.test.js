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

test('the badge never contradicts the server when both limits are set', () => {
	const done = 'All done for today';
	assert.equal(badge({ videos: 1, minutes: 0, ve_minutes: 15 }, 3)?.text, done);
	assert.equal(badge({ videos: 0, minutes: 10, ve_minutes: 15 }, 3)?.text, done);
	assert.equal(badge({ videos: 1.5, minutes: 0.3, ve_minutes: 15 }, 3)?.text, done);
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
	assert.equal(sections[0].title, 'New');
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

test('the grace countdown reads in plain words', () => {
	assert.equal(graceText(120), '2 minutes to look around');
	assert.equal(graceText(61), '2 minutes to look around');
	assert.equal(graceText(60), '60 seconds to look around');
	assert.equal(graceText(1), '1 second to look around');
	assert.equal(graceText(-5), '0 seconds to look around');
});
