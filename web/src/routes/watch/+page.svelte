<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import {
		api,
		badge,
		BEAT_MS,
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
		parseSearch
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

	// The watch page: idle | loading | playing | ended | refused | stopped
	let view = $state<'grid' | 'loading' | 'playing' | 'ended' | 'refused' | 'stopped'>('grid');
	let current = $state<Tile | null>(null);
	let refusal = $state('');
	let paused = $state(false);
	let playing = $state(false);
	let pos = $state(0);
	let dur = $state(0);
	let poll: ReturnType<typeof setInterval> | undefined;
	let suggestions = $state<Tile[]>([]);
	let sent = $state<Record<string, boolean>>({});
	let host: HTMLDivElement | undefined = $state();

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
	}

	onMount(() => {
		paired = !!token.get();
		if (paired) {
			loadHome();
			if (urlQuery) runSearch(urlQuery);
		}
		const leave = () => leaveWatch();
		addEventListener('pagehide', leave);
		return () => {
			removeEventListener('pagehide', leave);
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

	// The URL is the search state: /watch is home, /watch?q=... is a search, so back leaves it.
	const urlQuery = $derived(page.url.searchParams.get('q')?.trim() ?? '');
	let searchSeq = 0;

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

	function destroyPlayer() {
		clearInterval(timer);
		timer = undefined;
		clearInterval(poll);
		poll = undefined;
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

	function tickClock() {
		try {
			pos = player.getCurrentTime() ?? pos;
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
		player?.seekTo?.(v, true);
	}

	async function resume() {
		paused = false;
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
					onReady: () => {},
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
			paused = false;
			playing = true;
			clearInterval(poll);
			poll = setInterval(tickClock, 500);
			lastBeat = Date.now();
			clearInterval(timer);
			timer = setInterval(() => beat('playing'), BEAT_MS);
		} else if (s === S.PAUSED) {
			clearInterval(timer);
			timer = undefined;
			playing = false;
			tickClock();
			paused = true;
			beat('paused');
		} else if (s === S.ENDED) {
			playing = false;
			finish('ended', 'ended');
		}
	}

	async function ask(kind: 'like' | 'more-from-channel') {
		if (!current) return;
		const key = `${kind}:${current.youtube_id}`;
		sent[key] = true;
		await api('/api/child/request', { body: { kind, youtube_id: current.youtube_id } });
	}
</script>

<svelte:head><title>Cubby</title></svelte:head>

{#snippet tile(t: Tile)}
	<button class="tile" onclick={() => open(t)}>
		{#if thumb(t)}<img class="thumb" src={thumb(t)} alt="" />{:else}<span class="thumb blank"></span>{/if}
		<span class="meta">
			{#if localUrl(t.channel_avatar, origin)}<img class="avatar" src={localUrl(t.channel_avatar, origin)} alt="" />{/if}
			<span class="title">{t.title}</span>
		</span>
	</button>
{/snippet}

<main>
	{#if !paired}
		<form class="card" onsubmit={pair}>
			<h1>Hi! Type your code</h1>
			<input aria-label="pairing code" bind:value={code} inputmode="numeric" autocomplete="off" />
			<button type="submit">Go</button>
			{#if pairError}<p role="alert">{pairError}</p>{/if}
		</form>
	{:else}
		<header>
			{#if child.name}<span class="hello" data-testid="greeting"><span class="me">{child.avatar}</span> Hi, {child.name}!</span>{/if}
			{#if view !== 'grid'}<button class="back" onclick={back}>← Back</button>{/if}
			{#if badgeInfo}
				<div class="badge" data-testid="badge">
					<span>{badgeInfo.text}</span>
					<span class="meter"><span class="fill" style="width:{Math.round(badgeInfo.frac * 100)}%"></span></span>
				</div>
			{/if}
		</header>

		{#if error}<p role="alert">{error}</p>{/if}

		{#if view === 'grid'}
			<form class="search" role="search" onsubmit={search}>
				<span class="box">
					<input type="search" aria-label="search" placeholder="Search" bind:value={query} oninput={onSearchInput} />
					{#if query}<button type="button" class="clear" aria-label="Clear" onclick={home}>✕</button>{/if}
				</span>
				<button type="submit">Search</button>
			</form>
			{#if urlQuery}
				<div class="searched">
					<button type="button" class="home" onclick={home}>← Home</button>
					<h2>Videos about {urlQuery}</h2>
				</div>
			{/if}
			{#if results && searched === urlQuery}
				<section>
					{#if results.length === 0}<p>Nothing found.</p>{/if}
					<div class="grid">{#each results as t (t.youtube_id)}{@render tile(t)}{/each}</div>
				</section>
			{:else}
				{#if sections.every((s) => s.tiles.length === 0)}
					<p class="empty" data-testid="nothing-allowed">Nothing is allowed yet. Ask a grown-up to add some videos.</p>
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
			<p>One moment…</p>
		{:else if view === 'refused'}
			<div class="card" data-testid="refusal">
				<h1>{refusal}</h1>
				<button onclick={back}>OK</button>
			</div>
		{:else if view === 'stopped'}
			<div class="card" data-testid="stopped">
				<h1>All done for now</h1>
				<button onclick={back}>OK</button>
			</div>
		{:else if view === 'ended'}
			<div class="card" data-testid="end-card">
				<h1>That was fun!</h1>
				{#if badgeInfo}<p data-testid="end-budget">{badgeInfo.text}</p>{/if}
				<div class="grid">{#each suggestions as t (t.youtube_id)}{@render tile(t)}{/each}</div>
				<button onclick={back}>Back to all videos</button>
			</div>
		{:else if view === 'playing' && current}
			<h1 class="now">{current.title}</h1>
			<div class="stage">
				<div class="player" class:hidden={paused} bind:this={host}></div>
				{#if paused}
					<div class="pause" data-testid="pause-card">
						<h2>Taking a break?</h2>
						<button data-testid="resume" onclick={resume}>Keep watching</button>
						<button onclick={back}>All done</button>
					</div>
				{/if}
			</div>
			<div class="controls" data-testid="controls">
				<button class="big" data-testid="playpause" aria-label={playing ? 'Pause' : 'Play'} onclick={toggle}>{playing ? '⏸' : '▶'}</button>
				<input type="range" aria-label="seek" min="0" max={dur || 0} step="1" value={pos} oninput={seek} />
				<span data-testid="clock">{clock(pos)} / {dur ? `-${clock(dur - pos)}` : '--:--'}</span>
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
	{/if}
</main>

<style>
	main { max-width: 72rem; margin: 0 auto; padding: 1rem; font-family: system-ui, sans-serif; font-size: 1.25rem; }
	.hello { font-size: 1.2rem; font-weight: 600; }
	.me { font-size: 1.5rem; }
	header { display: flex; gap: 1rem; align-items: center; margin-bottom: 1rem; }
	.badge { display: flex; flex-direction: column; gap: 0.25rem; margin-left: auto; font-weight: 600; }
	.meter { display: block; width: 14rem; height: 0.75rem; background: #ddd; border-radius: 1rem; overflow: hidden; }
	.fill { display: block; height: 100%; background: #2a9d4b; }
	button { font-size: 1.25rem; padding: 0.75rem 1.25rem; border-radius: 0.75rem; border: 2px solid #345; background: #fff; }
	.search { display: flex; gap: 0.5rem; margin-bottom: 1rem; }
	.box { flex: 1; position: relative; display: flex; }
	.clear { position: absolute; right: 0.25rem; top: 50%; transform: translateY(-50%); border: none; background: none; padding: 0.5rem 0.75rem; }
	.searched { display: flex; gap: 1rem; align-items: center; margin-bottom: 1rem; }
	.searched h2 { margin: 0; }
	.search input { flex: 1; font-size: 1.5rem; padding: 0.75rem; }
	.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr)); gap: 1rem; }
	.tile { display: flex; flex-direction: column; text-align: left; padding: 0; overflow: hidden; }
	.thumb { width: 100%; aspect-ratio: 16 / 9; object-fit: cover; background: #ccd; }
	.meta { display: flex; gap: 0.5rem; padding: 0.5rem; }
	.avatar { width: 2rem; height: 2rem; border-radius: 50%; }
	.title { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
	.card { text-align: center; padding: 2rem; }
	.stage { position: relative; aspect-ratio: 16 / 9; background: #000; }
	.hidden { display: none; }
	.stage:has(.hidden) { background: #fff; }
	.controls { display: flex; gap: 1rem; align-items: center; margin-top: 1rem; }
	.controls input { flex: 1; height: 2.5rem; }
	.big { font-size: 2rem; min-width: 5rem; min-height: 4rem; }
	.player, .player :global(iframe) { width: 100%; height: 100%; }
	.pause { position: absolute; inset: 0; background: #fff; display: flex; flex-direction: column; gap: 1rem; align-items: center; justify-content: center; }
	.actions { display: flex; gap: 1rem; margin-top: 1rem; }
</style>
