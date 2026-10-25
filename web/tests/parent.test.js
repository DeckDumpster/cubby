import test from 'node:test';
import assert from 'node:assert/strict';
import { windowsFromText, windowsToText, fmtWatched, budgetBody, requestLabel, rulesBody, phraseFromText, withPhrase } from '../src/lib/parent.js';

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
			minutes_cap: null,
			ve_minutes: null,
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

test('loginHref returns to the given path', async () => {
	const { loginHref } = await import('../src/lib/parent.js');
	assert.equal(loginHref('/parent/children?x=1'), '/auth/login?next=%2Fparent%2Fchildren%3Fx%3D1');
});

test('the minutes per video equivalent ride the budget body; zero and junk are refused', () => {
	const f = { videos: '3', veMinutes: '10', minutes: '', windows: '', cutoff: false };
	assert.equal(budgetBody(f)?.ve_minutes, 10);
	assert.equal(budgetBody({ ...f, veMinutes: '0' }), null);
	assert.equal(budgetBody({ ...f, veMinutes: 'x' }), null);
});

const rf = { exclude_shorts: true, min: '', max: '30', has: [], lacks: ['compilation', 'how to, really'] };

test('rulesBody turns the form into the request body', () => {
	assert.deepEqual(rulesBody(rf), {
		exclude_shorts: true,
		min_minutes: null,
		max_minutes: 30,
		title_has: [],
		title_lacks: ['compilation', 'how to, really']
	});
});

test('rulesBody refuses a bad or inverted range', () => {
	assert.equal(rulesBody({ ...rf, max: 'abc' }), null);
	assert.equal(rulesBody({ ...rf, min: '40' }), null);
});

test('a quoted phrase is the same phrase as the bare one, and commas stay inside it', () => {
	assert.equal(phraseFromText('"how to"'), 'how to');
	assert.equal(phraseFromText('  how to '), 'how to');
	assert.equal(phraseFromText('\u201chow to\u201d'), 'how to');
	assert.equal(phraseFromText('a, b'), 'a, b');
	assert.equal(phraseFromText('""'), '');
});

test('withPhrase rejects empties and repeats ignoring case', () => {
	assert.deepEqual(withPhrase([], '"How To"'), ['How To']);
	assert.deepEqual(withPhrase(['How To'], 'how to'), ['How To']);
	assert.deepEqual(withPhrase(['a'], '  '), ['a']);
});
