/** Helpers for the parent screens: pure, so they are unit-tested without a browser. */

const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;

/**
 * "07:00-12:00, 14:00-19:30" to [["07:00","12:00"], ...]; null when it does not parse.
 * @param {string} text
 * @returns {[string, string][] | null}
 */
export function windowsFromText(text) {
	if (!text.trim()) return [];
	/** @type {[string, string][]} */
	const out = [];
	for (const part of text.split(',')) {
		const [a, b, ...rest] = part.trim().split('-');
		if (rest.length || !HHMM.test(a ?? '') || !HHMM.test(b ?? '')) return null;
		out.push([a, b]);
	}
	return out;
}

/** @param {[string, string][]} windows */
export function windowsToText(windows) {
	return windows.map(([a, b]) => `${a}-${b}`).join(', ');
}

/** @param {number} s */
export function fmtWatched(s) {
	const t = Math.round(s);
	if (t < 60) return `${t}s`;
	return `${Math.floor(t / 60)}m ${String(t % 60).padStart(2, '0')}s`;
}

/**
 * @param {{videos: string, minutes: string, windows: string, cutoff: boolean}} form
 * @returns {{video_equivalents: number|null, window_s: number, minutes_cap: number|null,
 *   cutoff_mid_play: boolean, daily_windows: [string, string][]} | null} null when a field is junk
 */
export function budgetBody(form) {
	const num = (/** @type {string} */ v) => (v.trim() === '' ? null : Number(v));
	const videos = num(form.videos);
	const minutes = num(form.minutes);
	const windows = windowsFromText(form.windows);
	if (windows === null) return null;
	if ([videos, minutes].some((n) => n !== null && (!Number.isFinite(n) || n < 0))) return null;
	return {
		video_equivalents: videos,
		window_s: 86400,
		minutes_cap: minutes,
		cutoff_mid_play: form.cutoff,
		daily_windows: windows
	};
}

/** @param {string} kind */
export function requestLabel(kind) {
	return kind === 'like' ? 'liked this video' : 'wants more from this channel';
}

/**
 * JSON call to the parent API; throws the server's error message on a non-2xx.
 * @param {string} method
 * @param {string} path
 * @param {unknown} [body]
 */
export async function api(method, path, body) {
	const res = await fetch(path, {
		method,
		cache: 'no-store',
		headers: body === undefined ? {} : { 'content-type': 'application/json' },
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	const data = await res.json().catch(() => ({}));
	if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
	return data;
}
