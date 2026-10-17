import test from 'node:test';
import assert from 'node:assert/strict';
import { windowsFromText, windowsToText, fmtWatched, budgetBody, requestLabel } from '../src/lib/parent.js';

test('windows text round-trips', () => {
	const w = windowsFromText('07:00-12:00, 14:00-19:30');
	assert.deepEqual(w, [['07:00', '12:00'], ['14:00', '19:30']]);
	assert.equal(windowsToText(w), '07:00-12:00, 14:00-19:30');
});

test('empty windows text means no windows, junk is null', () => {
	assert.deepEqual(windowsFromText('  '), []);
	assert.equal(windowsFromText('7am-9am'), null);
	assert.equal(windowsFromText('07:00'), null);
});

test('watched seconds read as minutes and seconds', () => {
	assert.equal(fmtWatched(0), '0s');
	assert.equal(fmtWatched(45), '45s');
	assert.equal(fmtWatched(125), '2m 05s');
});

test('a budget form becomes the API body, blanks becoming no limit', () => {
	assert.deepEqual(
		budgetBody({ videos: '3', minutes: '', windows: '07:00-19:00', cutoff: true }),
		{
			video_equivalents: 3,
			window_s: 86400,
			minutes_cap: null,
			cutoff_mid_play: true,
			daily_windows: [['07:00', '19:00']]
		}
	);
	assert.equal(budgetBody({ videos: 'x', minutes: '', windows: '', cutoff: false }), null);
	assert.equal(budgetBody({ videos: '', minutes: '', windows: 'nope', cutoff: false }), null);
});

test('request kinds have parent-facing labels', () => {
	assert.match(requestLabel('like'), /liked/);
	assert.match(requestLabel('more-from-channel'), /more/);
});
