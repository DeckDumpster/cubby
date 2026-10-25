/** Pure helpers and the API client for the child UI (`/watch`). */

export const PLAYER_HOST = 'https://www.youtube-nocookie.com';
export const PLAYER_VARS = { autoplay: 1, rel: 0, playsinline: 1, fs: 0, iv_load_policy: 3, controls: 0, disablekb: 1 };
export const BEAT_MS = 10_000;
/** The embed shows its own chrome after every start and seek; keep it hidden until PLAYING has lasted this long. Tune on an iPad. */
export const SETTLE_MS = 4000;
/** After the player is ready, how long to wait for PLAYING before treating autoplay as refused and showing the Play control. */
export const AUTOPLAY_WAIT_MS = 3000;
/** A playback position jump larger than this (seconds) between clock ticks is a seek we did not make. */
export const SEEK_JUMP_S = 2;
const TOKEN_KEY = 'cubby_token';

/** @param {string | undefined} ua */
export function isShell(ua) {
	return /CubbyShell\//.test(ua ?? '');
}

/**
 * @param {{ videos: number | null, minutes: number | null, ve_minutes?: number | null }} remaining
 * @param {number | null} total videos in a full budget, for the meter
 */
export function badge(remaining, total) {
	const { videos, minutes, ve_minutes } = remaining;
	const v = videos != null ? Math.max(0, Math.floor(videos)) : null;
	const m = minutes != null ? Math.max(0, Math.round(minutes)) : null;
	if (v === 0 || m === 0) return { text: 'All done for today', frac: 0 };
	const minutesBind = m != null && (v == null || (ve_minutes != null && ve_minutes > 0 && minutes != null && minutes < (videos ?? 0) * ve_minutes));
	if (minutesBind) return { text: `${m} minute${m === 1 ? '' : 's'} left today`, frac: 1 };
	if (v != null) return { text: `${v} video${v === 1 ? '' : 's'} left today`, frac: total ? Math.min(1, v / total) : 1 };
	return null;
}

/** @param {number} s seconds of grace left */
export function graceText(s) {
	const n = Math.max(0, Math.ceil(s));
	if (n > 60) {
		const m = Math.ceil(n / 60);
		return `${m} minute${m === 1 ? '' : 's'} to look around`;
	}
	return `${n} second${n === 1 ? '' : 's'} to look around`;
}

/**
 * @param {string} reason
 * @param {number | null | undefined} nextStart unix seconds
 */
export function refusalMessage(
	reason,
	nextStart,
	/** @type {string | undefined} */ locale = undefined,
	/** @type {string | undefined} */ timeZone = undefined
) {
	switch (reason) {
		case 'out-of-videos':
			return 'You have watched all your videos for today. See you next time!';
		case 'out-of-minutes':
			return 'Your watching time is up for today. See you next time!';
		case 'not-now': {
			if (!nextStart) return 'It is not video time right now.';
			const at = new Date(nextStart * 1000).toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit', timeZone });
			return `It is not video time right now. Come back at ${at}.`;
		}
		default:
			return 'This video is not available.';
	}
}

/** A url is kept only when it stays on `base`'s origin; the child DOM never points off it. */
/**
 * @param {string | undefined} url
 * @param {string} base
 */
export function localUrl(url, base) {
	if (!url) return '';
	try {
		const u = new URL(url, base);
		return u.origin === new URL(base).origin ? url : '';
	} catch {
		return '';
	}
}

/**
 * @param {number} lastAt
 * @param {number} now
 */
export function beatDue(lastAt, now) {
	return now - lastAt >= BEAT_MS;
}

/** @param {string} r */
export function endReason(r) {
	return r === 'ended' || r === 'stop' || r === 'off-list' ? r : 'left';
}

/** Resolves the IFrame API: the injected one when present, else loaded from YouTube. */
/** @param {any} win */
export function loadYT(win) {
	if (win.YT?.Player) return Promise.resolve(win.YT);
	return new Promise((resolve, reject) => {
		const prev = win.onYouTubeIframeAPIReady;
		win.onYouTubeIframeAPIReady = () => {
			prev?.();
			resolve(win.YT);
		};
		const s = win.document.createElement('script');
		s.src = 'https://www.youtube.com/iframe_api';
		s.onerror = () => reject(new Error('player unavailable'));
		win.document.head.appendChild(s);
	});
}

export const token = {
	get: () => localStorage.getItem(TOKEN_KEY),
	set: (/** @type {string} */ t) => localStorage.setItem(TOKEN_KEY, t),
	clear: () => localStorage.removeItem(TOKEN_KEY)
};

/**
 * @param {string} path
 * @param {{ method?: string, body?: unknown, signal?: AbortSignal }} [opts]
 * @returns {Promise<{ status: number, data: any }>}
 */
export async function api(path, opts = {}) {
	const t = token.get();
	const res = await fetch(path, {
		method: opts.method ?? (opts.body === undefined ? 'GET' : 'POST'),
		headers: {
			...(t ? { authorization: `Bearer ${t}` } : {}),
			...(opts.body === undefined ? {} : { 'content-type': 'application/json' })
		},
		body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
		signal: opts.signal,
		cache: 'no-store',
		keepalive: true
	});
	let data = null;
	try {
		data = await res.json();
	} catch {
		/* an empty body */
	}
	return { status: res.status, data };
}

/** The home payload as the page uses it: `sections[].tiles`, the one field name the server sends. */
export function parseHome(/** @type {any} */ data) {
	const sections = (data?.sections ?? []).map((/** @type {any} */ s) => ({
		key: String(s.key),
		title: s.title ?? String(s.key),
		tiles: s.tiles ?? []
	}));
	return { sections };
}

/** Search and suggestion payloads: `tiles`. */
export function parseSearch(/** @type {any} */ data) {
	return data?.tiles ?? [];
}

/** The history payload: `tiles` for this page, `more` when another page follows. */
export function parseRecent(/** @type {any} */ data) {
	return { tiles: data?.tiles ?? [], more: !!data?.more };
}

/** @param {number} s seconds */
export function clock(s) {
	const n = Math.max(0, Math.floor(Number.isFinite(s) ? s : 0));
	return `${Math.floor(n / 60)}:${String(n % 60).padStart(2, '0')}`;
}

/** True when the embed is showing a video other than the granted one. */
export function offList(/** @type {any} */ player, /** @type {string} */ granted) {
	try {
		const id = player?.getVideoData?.()?.video_id;
		return !!id && id !== granted;
	} catch {
		return false;
	}
}
