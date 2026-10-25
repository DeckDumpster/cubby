<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		api,
		badge,
		BEAT_MS,
		SETTLE_MS,
		AUTOPLAY_WAIT_MS,
		SEEK_JUMP_S,
		clock,
	endReason,
		loadYT,
		localUrl,
		offList,
		PLAYER_HOST,
		PLAYER_VARS,
		refusalMessage,
		token,
		parseHome,
		parseSearch,
		parseRecent
	} from '$lib/child.js';

	type Tile = { youtube_id: string; title: string; channel_title?: string; channel_avatar?: string; thumb?: string };
	type Remaining = { videos: number | null; minutes: number | null };

	const SHOWN = 8;
	const origin = typeof location === 'undefined' ? 'http://localhost' : location.origin;

	let paired = $state(true);
	let code = $state('');
	let pairError = $state('');
	let child = $state<{ name?: string; avatar?: string }>({});
	let remaining = $state<Remaining>({ videos: null, minutes: null });
	let total = $state<number | null>(null);
	let sections = $state<{ key: string; title: string; tiles: Tile[] }[]>([]);
	let expanded = $state<Record<string, boolean>>({});
	let query = $state('');
	let results = $state<Tile[] | null>(null);
	let searched = $state('');
	let error = $state('');
	// "Recently watched": a one-row preview on home, the full paged list at /watch?recent=1.
	let recentRow = $state<Tile[]>([]);
	let recentRowMore = $state(false);
	let recentAll = $state<Tile[]>([]);
	let recentMore = $state(false);
	let recentLoading = $state(false);

	// The watch page: idle | loading | playing | ended | refused | stopped
	let view = $state<'grid' | 'loading' | 'playing' | 'ended' | 'refused' | 'stopped'>('grid');
	let current = $state<Tile | null>(null);
	let refusal = $state('');
	let paused = $state(false);
	let playing = $state(false);
	// False from every start, resume and seek until PLAYING has lasted SETTLE_MS: the iframe stays visibility:hidden meanwhile.
	let settled = $state(false);
	// True once the browser refused to autoplay: the ready frame clears and the Play control is what is left.
	let blocked = $state(false);
	let blockTimer: ReturnType<typeof setTimeout> | undefined;
	let settleTimer: ReturnType<typeof setTimeout> | undefined;
	let lastPos = 0;
	let pos = $state(0);
	let dur = $state(0);
	let poll: ReturnType<typeof setInterval> | undefined;
	let suggestions = $state<Tile[]>([]);
	let sent = $state<Record<string, boolean>>({});
	let host: HTMLDivElement | undefined = $state();
	// Fullscreen is Cubby's own container, never the YouTube iframe. 'api' = element fullscreen,
	// 'fill' = the CSS fixed-viewport fallback (iPhone Safari, WKWebView without element fullscreen).
	let box: HTMLDivElement | undefined = $state();
	let fsMode = $state<'off' | 'api' | 'fill'>('off');

	let player: any = null;
	let grantId = '';
	let lastBeat = 0;
	let timer: ReturnType<typeof setInterval> | undefined;

	const badgeInfo = $derived(badge(remaining, total));

	function thumb(t: Tile) {
		return localUrl(t.thumb, origin);
	}

	async function guarded<T>(r: Promise<{ status: number; data: T }>) {
		const res = await r;
		if (res.status === 401) {
			token.clear();
			paired = false;
		}
		return res;
	}

	async function loadHome() {
		const { status, data } = await guarded(api('/api/child/home'));
		if (status !== 200) return;
		child = data.child ?? {};
		remaining = data.remaining ?? remaining;
		total = data.total_videos ?? total;
		sections = parseHome(data).sections;
		loadRecentRow();
	}

	async function loadRecentRow() {
		const { status, data } = await guarded(api('/api/child/recent?limit=12'));
		if (status !== 200) return;
		const r = parseRecent(data);
		recentRow = r.tiles;
		recentRowMore = r.more;
	}

	async function loadRecentPage(reset: boolean) {
		if (recentLoading) return;
		recentLoading = true;
		if (reset) recentAll = [];
		const { status, data } = await guarded(api(`/api/child/recent?limit=24&offset=${recentAll.length}`));
		recentLoading = false;
		if (status !== 200) return;
		const r = parseRecent(data);
		recentAll = [...recentAll, ...r.tiles];
		recentMore = r.more;
	}

	function seeAll() {
		goto('/watch?recent=1', { noScroll: false });
	}

	onMount(() => {
		paired = !!token.get();
		if (paired) {
			loadHome();
			if (urlQuery) runSearch(urlQuery);
		}
		const leave = () => leaveWatch();
		const fsChange = () => {
			if (fsMode === 'api' && !fsElement()) fsMode = 'off';
		};
		addEventListener('pagehide', leave);
		document.addEventListener('fullscreenchange', fsChange);
		document.addEventListener('webkitfullscreenchange', fsChange);
		return () => {
			removeEventListener('pagehide', leave);
			document.removeEventListener('fullscreenchange', fsChange);
			document.removeEventListener('webkitfullscreenchange', fsChange);
			exitFullscreen();
			destroyPlayer();
		};
	});

	async function pair(e: Event) {
		e.preventDefault();
		pairError = '';
		const { status, data } = await api('/api/child/pair', { body: { code: code.trim(), label: navigator.userAgent.slice(0, 60) } });
		if (status === 200 && data?.token) {
			token.set(data.token);
			paired = true;
			await loadHome();
			if (urlQuery) runSearch(urlQuery);
		} else {
			pairError = 'That code did not work. Ask a grown-up for a new one.';
		}
	}

	function unpair() {
		if (!confirm('Unpair this device? A grown-up will need to make a new code.')) return;
		token.clear();
		paired = false;
		sections = [];
		results = null;
		code = '';
	}

	// The URL is the search state: /watch is home, /watch?q=... is a search, so back leaves it.
	const urlQuery = $derived(page.url.searchParams.get('q')?.trim() ?? '');
	const showRecent = $derived(page.url.searchParams.get('recent') === '1' && !urlQuery);
	let searchSeq = 0;

	$effect(() => {
		if (showRecent && untrack(() => paired)) loadRecentPage(true);
	});

	async function runSearch(q: string) {
		const seq = ++searchSeq;
		query = q;
		if (!q) {
			results = null;
			searched = '';
			return;
		}
		const { status, data } = await guarded(api(`/api/child/search?q=${encodeURIComponent(q)}`));
		if (seq !== searchSeq) return;
		searched = q;
		results = status === 200 ? parseSearch(data) : [];
	}

	$effect(() => {
		const q = urlQuery;
		if (untrack(() => paired)) runSearch(q);
	});

	function search(e: Event) {
		e.preventDefault();
		const q = query.trim();
		if (q === urlQuery) return;
		goto(q ? `/watch?q=${encodeURIComponent(q)}` : '/watch', { keepFocus: true, noScroll: true });
	}

	function home() {
		query = '';
		if (urlQuery) goto('/watch', { noScroll: true });
	}

	function onSearchInput() {
		if (!query.trim() && urlQuery) home();
	}

	function fsElement(): Element | null {
		const d = document as any;
		return d.fullscreenElement ?? d.webkitFullscreenElement ?? null;
	}

	function fsEnabled() {
		const d = document as any;
		return !!(d.fullscreenEnabled || d.webkitFullscreenEnabled) && !!(box && ((box as any).requestFullscreen || (box as any).webkitRequestFullscreen));
	}

	async function enterFullscreen() {
		if (!box) return;
		if (fsEnabled()) {
			try {
				const b = box as any;
				await (b.requestFullscreen ? b.requestFullscreen() : b.webkitRequestFullscreen());
				fsMode = 'api';
				return;
			} catch {
				/* refused: use the fill fallback */
			}
		}
		fsMode = 'fill';
	}

	function exitFullscreen() {
		if (fsMode === 'api' && fsElement()) {
			const d = document as any;
			try {
				(d.exitFullscreen ?? d.webkitExitFullscreen)?.call(d);
			} catch {
				/* already out */
			}
		}
		fsMode = 'off';
	}

	function toggleFullscreen() {
		if (fsMode === 'off') enterFullscreen();
		else exitFullscreen();
	}

	function destroyPlayer() {
		clearInterval(timer);
		timer = undefined;
		clearInterval(poll);
		poll = undefined;
		clearTimeout(settleTimer);
		settleTimer = undefined;
		settled = false;
		clearTimeout(blockTimer);
		blockTimer = undefined;
		blocked = false;
		lastPos = 0;
		try {
			player?.destroy();
		} catch {
			/* already gone */
		}
		player = null;
		paused = false;
		playing = false;
		pos = 0;
		dur = 0;
	}

	async function beat(state: string) {
		if (!grantId || !player) return;
		lastBeat = Date.now();
		let position = 0;
		try {
			position = player.getCurrentTime();
		} catch {
			/* not ready */
		}
		const { status, data } = await api(`/api/child/grant/${grantId}/beat`, { body: { position_s: position, state } });
		if (status !== 200) return;
		remaining = data.remaining ?? remaining;
		if (data.stop) await finish('stop', 'stopped');
	}

	async function closeGrant(reason: string) {
		const id = grantId;
		grantId = '';
		if (id) await api(`/api/child/grant/${id}/end`, { body: { reason: endReason(reason) } });
	}

	function unsettle() {
		settled = false;
		clearTimeout(settleTimer);
		settleTimer = undefined;
		if (playing) armSettle();
	}

	function armSettle() {
		if (settled || settleTimer) return;
		settleTimer = setTimeout(() => {
			settleTimer = undefined;
			settled = true;
		}, SETTLE_MS);
	}

	function tickClock() {
		try {
			const t = player.getCurrentTime() ?? pos;
			if (Math.abs(t - lastPos) > SEEK_JUMP_S) unsettle();
			lastPos = t;
			pos = t;
			dur = player.getDuration?.() || dur;
		} catch {
			/* not ready */
		}
	}

	function toggle() {
		if (!player) return;
		if (playing) player.pauseVideo?.();
		else player.playVideo?.();
	}

	function seek(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		pos = v;
		lastPos = v;
		unsettle();
		player?.seekTo?.(v, true);
	}

	async function resume() {
		paused = false;
		unsettle();
		await tick();
		player?.playVideo?.();
	}

	async function finish(reason: string, next: 'ended' | 'stopped') {
		const id = current?.youtube_id;
		await beat('ended').catch(() => {});
		destroyPlayer();
		await closeGrant(reason);
		view = next;
		if (id && next === 'ended') {
			const { status, data } = await api(`/api/child/next?after=${encodeURIComponent(id)}`);
			if (status === 200) {
				suggestions = parseSearch(data);
				remaining = data.remaining ?? remaining;
			}
		}
	}

	async function leaveWatch() {
		if (view !== 'playing') return;
		await beat('closed').catch(() => {});
		destroyPlayer();
		await closeGrant('left');
	}

	async function back() {
		exitFullscreen();
		await leaveWatch();
		view = 'grid';
		current = null;
		await loadHome();
	}

	async function open(tile: Tile) {
		await leaveWatch();
		current = tile;
		view = 'loading';
		const { status, data } = await guarded(api('/api/child/play', { body: { youtube_id: tile.youtube_id } }));
		if (status !== 200) {
			error = 'Something went wrong. Try again.';
			view = 'grid';
			return;
		}
		if (data.refused) {
			refusal = refusalMessage(data.refused, data.next_start);
			view = 'refused';
			return;
		}
		grantId = data.grant_id;
		remaining = data.remaining ?? remaining;
		view = 'playing';
		await tick();
		try {
			const YT = await loadYT(window as any);
			const mount = document.createElement('div');
			host!.replaceChildren(mount);
			player = new YT.Player(mount, {
				host: PLAYER_HOST,
				videoId: tile.youtube_id,
				playerVars: { ...PLAYER_VARS, origin },
				events: {
					onReady: () => {
						player?.playVideo?.();
						clearTimeout(blockTimer);
						blockTimer = setTimeout(() => {
							if (!playing) blocked = true;
						}, AUTOPLAY_WAIT_MS);
					},
					onAutoplayBlocked: () => {
						blocked = true;
					},
					onStateChange: (e: { data: number }) => onState(YT, e.data)
				}
			});
		} catch {
			await closeGrant('left');
			error = 'The video player could not start.';
			view = 'grid';
		}
	}

	function onState(YT: any, s: number) {
		const S = YT.PlayerState;
		if (grantId && current && offList(player, current.youtube_id)) {
			finish('off-list', 'stopped');
			return;
		}
		if (s === S.PLAYING) {
			clearTimeout(blockTimer);
			blockTimer = undefined;
			blocked = false;
			paused = false;
			playing = true;
			armSettle();
			clearInterval(poll);
			poll = setInterval(tickClock, 500);
			lastBeat = Date.now();
			clearInterval(timer);
			timer = setInterval(() => beat('playing'), BEAT_MS);
		} else if (s === S.PAUSED) {
			clearTimeout(settleTimer);
			settleTimer = undefined;
			settled = false;
			clearInterval(timer);
			timer = undefined;
			playing = false;
			tickClock();
			paused = true;
			beat('paused');
		} else if (s === S.ENDED) {
			playing = false;
			finish('ended', 'ended');
		} else {
			// buffering, cued, unstarted: the chrome may be up, restart the count
			clearTimeout(settleTimer);
			settleTimer = undefined;
			settled = false;
		}
	}

	async function ask(kind: 'like' | 'more-from-channel') {
		if (!current) return;
		const key = `${kind}:${current.youtube_id}`;
		sent[key] = true;
		await api('/api/child/request', { body: { kind, youtube_id: current.youtube_id } });
	}
</script>

<svelte:head><title>{view === 'playing' && current ? `${current.title} · Cubby` : 'Cubby'}</title></svelte:head>

{#snippet tile(t: Tile)}
	<button class="tile" onclick={() => open(t)}>
		{#if thumb(t)}<img class="thumb" src={thumb(t)} alt="" />{:else}<span class="thumb blank"></span>{/if}
		<span class="meta">
			{#if localUrl(t.channel_avatar, origin)}<img class="avatar" src={localUrl(t.channel_avatar, origin)} alt="" />{/if}
			<span class="title">{t.title}</span>
		</span>
	</button>
{/snippet}

<main class="watch">
	{#if !paired}
		<form class="card pair" onsubmit={pair}>
			<h1>Hi! Type your code</h1>
			<input
				class="code"
				aria-label="pairing code"
				bind:value={code}
				oninput={() => (code = code.toUpperCase().replace(/[^A-Z0-9]/g, ''))}
				maxlength="6"
				inputmode="text"
				autocapitalize="characters"
				autocorrect="off"
				spellcheck="false"
				autocomplete="off"
				placeholder="······"
			/>
			<button class="primary go" type="submit">Go</button>
			{#if pairError}<p class="error" role="alert">{pairError}</p>{/if}
		</form>
	{:else}
		<header>
			<span class="brand"><span class="logo">C</span>Cubby</span>
			{#if child.name}<span class="hello" data-testid="greeting"><span class="me">{child.avatar}</span> Hi, {child.name}!</span>{/if}
			{#if view !== 'grid'}<button class="back" onclick={back}>← Back</button>{/if}
			{#if badgeInfo}
				<div class="badge" data-testid="badge">
					<span>{badgeInfo.text}</span>
					<span class="meter"><span class="fill" style="width:{Math.round(badgeInfo.frac * 100)}%"></span></span>
				</div>
			{/if}
		</header>

		{#if error}<p class="error" role="alert">{error}</p>{/if}

		{#if view === 'grid'}
			<form class="search" role="search" onsubmit={search}>
				<span class="box">
					<input type="search" aria-label="search" placeholder="Search" bind:value={query} oninput={onSearchInput} />
					{#if query}<button type="button" class="clear" aria-label="Clear" onclick={home}>✕</button>{/if}
				</span>
				<button class="primary" type="submit">Search</button>
			</form>
			{#if urlQuery}
				<div class="searched">
					<button type="button" class="home" onclick={home}>← Home</button>
					<h2>Videos about {urlQuery}</h2>
				</div>
			{/if}
			{#if results && searched === urlQuery}
				<section>
					{#if results.length === 0}<p class="empty">Nothing found.</p>{/if}
					<div class="grid">{#each results as t (t.youtube_id)}{@render tile(t)}{/each}</div>
				</section>
			{:else if showRecent}
				<div class="searched">
					<button type="button" class="home" onclick={() => goto('/watch')}>← Home</button>
					<h2>Recently watched</h2>
				</div>
				{#if recentAll.length === 0 && !recentLoading}<p class="empty">Nothing watched yet.</p>{/if}
				<div class="grid" data-testid="recent-all">{#each recentAll as t (t.youtube_id)}{@render tile(t)}{/each}</div>
				{#if recentMore}<button class="more" disabled={recentLoading} onclick={() => loadRecentPage(false)}>More</button>{/if}
			{:else}
				{#if sections.every((s) => s.tiles.length === 0) && recentRow.length === 0}
					<p class="empty" data-testid="nothing-allowed">Nothing is allowed yet. Ask a grown-up to add some videos.</p>
				{/if}
				{#if recentRow.length}
					<section data-testid="recent-row">
						<div class="searched">
							<h2>Recently watched</h2>
							<button class="seeall" onclick={seeAll}>See all</button>
						</div>
						<div class="rail">{#each recentRow as t (t.youtube_id)}{@render tile(t)}{/each}</div>
					</section>
				{/if}
				{#each sections as s (s.key)}
					{#if s.tiles.length}
						<section>
							<h2>{s.title}</h2>
							<div class="grid">
								{#each expanded[s.key] ? s.tiles : s.tiles.slice(0, SHOWN) as t (t.youtube_id)}{@render tile(t)}{/each}
							</div>
							{#if !expanded[s.key] && s.tiles.length > SHOWN}
								<button class="more" onclick={() => (expanded[s.key] = true)}>More</button>
							{/if}
						</section>
					{/if}
				{/each}
			{/if}
		{:else if view === 'loading'}
			<p class="empty">One moment…</p>
		{:else if view === 'refused'}
			<div class="card" data-testid="refusal">
				<h1>{refusal}</h1>
				<button class="primary" onclick={back}>OK</button>
			</div>
		{:else if view === 'stopped'}
			<div class="card" data-testid="stopped">
				<h1>All done for now</h1>
				<button class="primary" onclick={back}>OK</button>
			</div>
		{:else if view === 'ended' || (view === 'playing' && current)}
			<div class="fsbox" class:fs={fsMode !== 'off'} class:fill={fsMode === 'fill'} data-testid="fsbox" data-fs={fsMode} bind:this={box}>
				{#if view === 'ended'}
					<div class="card" data-testid="end-card">
						<h1>That was fun!</h1>
						{#if badgeInfo}<p data-testid="end-budget">{badgeInfo.text}</p>{/if}
						<div class="grid">{#each suggestions as t (t.youtube_id)}{@render tile(t)}{/each}</div>
						<button class="primary" onclick={back}>Back to all videos</button>
					</div>
				{:else if current}
					<h1 class="now">{current.title}</h1>
					<div class="stage">
						<div class="player" class:hidden={paused} class:unsettled={!settled && !blocked} data-settled={settled} bind:this={host}></div>
						<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
						<div class="shield" data-testid="shield" aria-hidden="true" onclick={toggle}></div>
						{#if !settled && !paused && !blocked}
							<div class="ready" data-testid="ready-frame"><h2 class="break">Getting your video ready…</h2></div>
						{/if}
						{#if paused}
							<div class="pause" data-testid="pause-card">
								<h2 class="break">Taking a break?</h2>
								<button class="primary" data-testid="resume" onclick={resume}>Keep watching</button>
								<button onclick={back}>All done</button>
							</div>
						{/if}
					</div>
					<div class="controls" data-testid="controls">
						<button class="big" data-testid="playpause" aria-label={playing ? 'Pause' : 'Play'} onclick={toggle}>{playing ? '⏸' : '▶'}</button>
						<input type="range" aria-label="seek" min="0" max={dur || 0} step="1" value={pos} oninput={seek} />
						<span data-testid="clock">{clock(pos)} / {dur ? `-${clock(dur - pos)}` : '--:--'}</span>
						<button class="big" data-testid="fullscreen" aria-pressed={fsMode !== 'off'} aria-label={fsMode === 'off' ? 'Full screen' : 'Exit full screen'} onclick={toggleFullscreen}>{fsMode === 'off' ? '⛶' : '✕'}</button>
					</div>
					<div class="actions">
						<button disabled={sent[`like:${current.youtube_id}`]} onclick={() => ask('like')}>
							{sent[`like:${current.youtube_id}`] ? 'Asked! 👍' : '👍 I like this'}
						</button>
						<button disabled={sent[`more-from-channel:${current.youtube_id}`]} onclick={() => ask('more-from-channel')}>
							{sent[`more-from-channel:${current.youtube_id}`] ? 'Asked!' : 'More from this channel'}
						</button>
					</div>
				{/if}
			</div>
		{/if}
	{/if}
	{#if paired && view === 'grid'}
		<footer><button class="unpair" onclick={unpair}>Grown-up: unpair this device</button></footer>
	{/if}
</main>

<style>
	.watch { max-width: 76rem; margin: 0 auto; padding: var(--space-4) var(--space-4) var(--space-5); font-size: 1.25rem; min-height: 100vh; }
	header { display: flex; gap: var(--space-4); align-items: center; margin-bottom: var(--space-4); }
	.brand { display: inline-flex; align-items: center; gap: var(--space-2); font-weight: 800; font-size: 1.5rem; }
	.logo { display: inline-grid; place-items: center; width: 2.5rem; height: 2.5rem; border-radius: 12px; background: var(--accent); color: var(--accent-text); font-weight: 800; }
	h1 { font-size: 2rem; margin: 0 0 var(--space-4); line-height: 1.2; }
	h2 { font-size: 1.6rem; margin: var(--space-5) 0 var(--space-3); }
	h1.now { font-size: 1.5rem; margin: 0 0 var(--space-3); }

	.badge { display: flex; flex-direction: column; gap: var(--space-1); margin-left: auto; font-weight: 700; background: var(--surface); border: 2px solid var(--border); border-radius: var(--radius); padding: var(--space-2) var(--space-4); }
	.meter { display: block; width: 14rem; max-width: 100%; height: 0.8rem; background: var(--surface-2); border: 1px solid var(--muted); border-radius: 1rem; overflow: hidden; }
	.fill { display: block; height: 100%; background: var(--ok); }

	button, input { font: inherit; color: var(--text); }
	button { min-height: 3.5rem; padding: 0 var(--space-5); border-radius: 14px; border: 2px solid var(--text); background: var(--surface); font-weight: 700; cursor: pointer; }
	button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
	button:disabled { background: var(--surface-2); color: var(--muted); border-color: var(--border); cursor: default; }
	:is(button, input):focus-visible { outline: 4px solid var(--accent); outline-offset: 3px; }

	.search { display: flex; gap: var(--space-3); margin-bottom: var(--space-4); }
	.search input { flex: 1; min-width: 0; min-height: 3.5rem; font-size: 1.4rem; padding: 0 var(--space-4); border: 2px solid var(--text); border-radius: 14px; background: var(--surface); }
	input::placeholder { color: var(--muted); opacity: 1; }

	.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr)); gap: var(--space-4); }
	.rail { display: grid; grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr)); grid-template-rows: auto; grid-auto-rows: 0; overflow: hidden; gap: var(--space-4); }
	.rail > :global(*) { min-width: 0; }
	.seeall { margin-left: auto; }
	.tile { display: flex; flex-direction: column; text-align: left; padding: 0; overflow: hidden; min-height: 0; border: 2px solid var(--border); background: var(--surface); box-shadow: var(--shadow); }
	.thumb { display: block; width: 100%; aspect-ratio: 16 / 9; object-fit: cover; background: var(--surface-2); }
	.meta { display: flex; gap: var(--space-2); align-items: flex-start; padding: var(--space-3); }
	.avatar { width: 2rem; height: 2rem; border-radius: 50%; flex: none; }
	.title { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: 1.1rem; line-height: 1.3; font-weight: 700; color: var(--text); }
	.more { margin-top: var(--space-4); }

	.card { background: var(--surface); border: 2px solid var(--border); border-radius: var(--radius); box-shadow: var(--shadow); text-align: center; padding: var(--space-5); margin: var(--space-4) auto; max-width: 56rem; }
	.card .grid { margin: var(--space-4) 0; text-align: left; }
	.pair { max-width: 30rem; margin-top: 8vh; display: grid; gap: var(--space-4); justify-items: stretch; }
	.pair h1 { margin: 0; }
	.code { min-height: 5rem; width: 100%; font-family: var(--mono); font-size: 2.75rem; font-weight: 700; letter-spacing: 0.35em; text-align: center; text-indent: 0.35em; text-transform: uppercase; padding: 0 var(--space-3); border: 3px solid var(--text); border-radius: 16px; background: var(--surface); }
	.go { min-height: 4.5rem; font-size: 1.75rem; width: 100%; }

	.empty { text-align: center; color: var(--text); background: var(--surface); border: 2px dashed var(--muted); border-radius: var(--radius); padding: var(--space-5); font-size: 1.4rem; }
	.error { background: var(--danger-bg); color: var(--danger); border: 2px solid var(--danger); border-radius: var(--radius); padding: var(--space-3) var(--space-4); margin: 0; font-weight: 600; }

	.stage { --chrome: 17rem; position: relative; width: min(100%, calc((100dvh - var(--chrome)) * 16 / 9)); margin: 0 auto; aspect-ratio: 16 / 9; background: #000; border-radius: var(--radius); overflow: hidden; }
	.player, .player :global(iframe) { width: 100%; height: 100%; border: 0; }
	.player { pointer-events: none; }
	.shield { position: absolute; inset: 0; touch-action: manipulation; cursor: default; }
	.player.unsettled { visibility: hidden; }
	.ready { position: absolute; inset: 0; background: var(--surface); display: flex; align-items: center; justify-content: center; }
	.pause { position: absolute; inset: 0; background: var(--surface); display: flex; flex-direction: column; gap: var(--space-4); align-items: center; justify-content: center; }
	.break { margin: 0; font-size: 2rem; }
	.actions { display: flex; flex-wrap: wrap; gap: var(--space-3); margin-top: var(--space-4); }

	footer { margin-top: var(--space-5); text-align: center; }
	.unpair { min-height: var(--touch); font-size: 0.9rem; font-weight: 500; padding: 0 var(--space-3); border: 0; background: transparent; color: var(--muted); text-decoration: underline; }

	@media (max-width: 700px) {
		.badge { margin-left: 0; }
		header { flex-wrap: wrap; }
		.stage { --chrome: 21rem; }
		.controls { gap: var(--space-2); }
		.controls .big { min-width: 3.5rem; padding: 0 var(--space-2); }
		.controls span { font-size: 0.95rem; }
	}
	.hello { font-size: 1.3rem; font-weight: 700; }
	.me { font-size: 1.6rem; }
	.box { flex: 1; position: relative; display: flex; }
	.box input { width: 100%; padding-right: 3.5rem; }
	.clear { position: absolute; right: 0.25rem; top: 50%; transform: translateY(-50%); min-height: var(--touch); min-width: var(--touch); padding: 0; border: 0; background: none; font-size: 1.25rem; }
	.searched { display: flex; gap: var(--space-4); align-items: center; margin-bottom: var(--space-4); flex-wrap: wrap; }
	.searched h2 { margin: 0; }
	.hidden { display: none; }
	.stage:has(.hidden) { background: var(--surface); }
	.controls { display: flex; gap: var(--space-4); align-items: center; margin-top: var(--space-4); }
	.controls input { flex: 1; min-width: 0; min-height: var(--touch); padding: 0; accent-color: var(--accent); }
	.controls span { font-variant-numeric: tabular-nums; font-weight: 700; }
	.fsbox.fs { background: var(--bg, var(--surface)); overflow: auto; padding: var(--space-3); box-sizing: border-box; display: flex; flex-direction: column; }
	.fsbox.fill { position: fixed; inset: 0; z-index: 1000; width: 100dvw; height: 100dvh; padding: max(var(--space-3), env(safe-area-inset-top)) max(var(--space-3), env(safe-area-inset-right)) max(var(--space-3), env(safe-area-inset-bottom)) max(var(--space-3), env(safe-area-inset-left)); }
	.fsbox.fs .stage { aspect-ratio: auto; flex: 1; min-height: 0; width: 100%; margin: 0; }
	.fsbox.fs h1.now, .fsbox.fs .actions { display: none; }
	.big { font-size: 2rem; min-width: 5rem; min-height: 4rem; }
</style>
