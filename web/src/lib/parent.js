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
 * @param {{videos: string, minutes: string, windows: string, cutoff: boolean, veMinutes?: string}} form
 * @returns {{video_equivalents: number|null, minutes_cap: number|null, ve_minutes: number|null,
 *   cutoff_mid_play: boolean, daily_windows: [string, string][]} | null} null when a field is junk
 */
export function budgetBody(form) {
	const num = (/** @type {string} */ v) => (v.trim() === '' ? null : Number(v));
	const videos = num(form.videos);
	const minutes = num(form.minutes);
	const veMinutes = num(form.veMinutes ?? '');
	const windows = windowsFromText(form.windows);
	if (windows === null) return null;
	if (veMinutes !== null && (!Number.isFinite(veMinutes) || veMinutes <= 0)) return null;
	if ([videos, minutes].some((n) => n !== null && (!Number.isFinite(n) || n < 0))) return null;
	return {
		video_equivalents: videos,
		minutes_cap: minutes,
		ve_minutes: veMinutes,
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
	/** @type {Response} */
	let res;
	try {
		res = await fetch(path, {
			method,
			cache: 'no-store',
			redirect: 'manual',
			headers: body === undefined ? {} : { 'content-type': 'application/json' },
			body: body === undefined ? undefined : JSON.stringify(body)
		});
	} catch (e) {
		// A same-origin API call that throws was redirected away (an expired Access session sends it
		// cross-origin), which is a sign-in problem, not a network one.
		if (e instanceof TypeError && signIn()) return new Promise(() => {});
		throw e;
	}
	if ((res.status === 401 || res.type === 'opaqueredirect') && signIn()) return new Promise(() => {});
	const data = await res.json().catch(() => ({}));
	if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
	return data;
}

/** Sends the browser to sign in and back to this page; false outside a /parent page. */
function signIn() {
	if (typeof location === 'undefined' || !location.pathname.startsWith('/parent')) return false;
	location.assign(loginHref(location.pathname + location.search));
	return true;
}

/** @param {string} back */
export function loginHref(back) {
	return `/auth/login?next=${encodeURIComponent(back)}`;
}

/** @param {string} id */
export function watchUrl(id) {
	return `https://www.youtube.com/watch?v=${encodeURIComponent(id)}`;
}

/**
 * @param {number} t unix seconds
 * @param {string} [tz]
 */
export function fmtTime(t, tz) {
	return new Date(t * 1000).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', timeZone: tz });
}

/** @param {string | null} reason */
export function endLabel(reason) {
	const words = /** @type {Record<string, string>} */ ({
		ended: 'Finished the video',
		left: 'Left the video',
		stop: 'Stopped it',
		'off-list': 'Taken off the allow-list',
		timeout: 'Went quiet, timed out'
	});
	return reason === null ? 'Still playing' : (words[reason] ?? reason);
}

/**
 * Plays grouped under one heading per local day in tz, newest day first.
 * @template {{granted_at: number}} T
 * @param {T[]} grants
 * @param {string | undefined} tz
 * @param {number} now unix seconds
 * @returns {{key: string, label: string, items: T[]}[]}
 */
export function dayGroups(grants, tz, now) {
	const key = (/** @type {number} */ t) => new Date(t * 1000).toLocaleDateString('en-CA', { timeZone: tz });
	const today = key(now);
	const yesterday = key(now - 86400);
	/** @type {Map<string, {key: string, label: string, items: T[]}>} */
	const out = new Map();
	for (const g of [...grants].sort((a, b) => b.granted_at - a.granted_at)) {
		const k = key(g.granted_at);
		if (!out.has(k)) {
			const label =
				k === today ? 'Today' : k === yesterday ? 'Yesterday'
				: new Date(g.granted_at * 1000).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: tz }).replace(',', '');
			out.set(k, { key: k, label, items: [] });
		}
		out.get(k)?.items.push(g);
	}
	return [...out.values()];
}

/**
 * "trailer, compilation" to ["trailer", "compilation"]: split on commas, trimmed, blanks dropped.
 * @param {string} text
 * @returns {string[]}
 */
export function wordsFromText(text) {
	return text.split(',').map((w) => w.trim()).filter(Boolean);
}

/**
 * A minutes field to a number, null when empty, undefined when it is not a number of minutes.
 * @param {string} text
 * @returns {number | null | undefined}
 */
export function minutesFromText(text) {
	if (!text.trim()) return null;
	const n = Number(text);
	return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/**
 * The form's fields as the rules request body; null when a minutes field does not parse.
 * @param {{ exclude_shorts: boolean, min: string, max: string, has: string, lacks: string }} f
 */
export function rulesBody(f) {
	const min_minutes = minutesFromText(f.min);
	const max_minutes = minutesFromText(f.max);
	if (min_minutes === undefined || max_minutes === undefined) return null;
	if (min_minutes !== null && max_minutes !== null && min_minutes > max_minutes) return null;
	return {
		exclude_shorts: f.exclude_shorts,
		min_minutes,
		max_minutes,
		title_has: wordsFromText(f.has),
		title_lacks: wordsFromText(f.lacks)
	};
}
