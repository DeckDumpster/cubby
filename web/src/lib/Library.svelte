<script lang="ts">
	import { untrack } from 'svelte';
	import { api, fmtWatched, fmtLength } from '$lib/parent.js';
	import ActionButton from '$lib/ActionButton.svelte';
	import { getParent } from '$lib/parentState.svelte.js';

	type Source = { policy_id: string; kind: string; youtube_id: string; count: number };
	type Group = { key: string; title: string; count: number; playlists: Source[] };
	type Vid = { youtube_id: string; title: string; channel_title: string; policy_id: string; plays: number; watched_s: number; duration_s: number | null };
	type Page = { videos: Vid[]; total: number; loading: boolean };

	let { view }: { view: 'allow' | 'review' } = $props();
	const ps = getParent();
	const PAGE = 100;

	let q = $state('');
	let child = $state('');
	let sort = $state('title');
	let groups: Group[] = $state([]);
	let loaded = $state(false);
	let open: Record<string, boolean> = $state({});
	let pages: Record<string, Page> = $state({});

	const sourceKey = (g: Group, s: Source) => `p:${g.key}|${s.policy_id}`;
	const groupTitle = (g: Group) => (g.key === 'direct' ? 'Allowed one by one' : g.title || 'Unknown channel');
	const sourceTitle = (s: Source) => (s.kind === 'channel' ? 'Every upload' : `Playlist ${s.youtube_id}`);

	function params(extra: Record<string, string> = {}) {
		const p = new URLSearchParams({ view, ...extra });
		if (q.trim()) p.set('q', q.trim());
		if (child) p.set('child', child);
		return p.toString();
	}

	async function fetchPage(key: string, group: string, policy: string | null, more = false) {
		const have = more ? (pages[key]?.videos ?? []) : [];
		pages[key] = { videos: have, total: pages[key]?.total ?? 0, loading: true };
		const extra: Record<string, string> = { group, sort, offset: String(have.length), limit: String(PAGE) };
		if (policy) extra.policy = policy;
		const r = await api('GET', `/api/parent/library/videos?${params(extra)}`);
		pages[key] = { videos: [...have, ...(r.videos ?? [])], total: r.total ?? 0, loading: false };
	}

	/** The key, group and policy a leaf list is fetched with: a group without playlists is a leaf itself. */
	function leaves(g: Group): [string, string, string | null][] {
		if (g.playlists.length === 0) return [[`g:${g.key}`, g.key, null]];
		return g.playlists.map((s) => [sourceKey(g, s), g.key, s.policy_id]);
	}

	async function refresh(autoOpen = false) {
		await ps.run(async () => {
			const r = await api('GET', `/api/parent/library?${params()}`);
			groups = r.groups ?? [];
			pages = {};
			if (autoOpen) {
				open = {};
				// Searching opens the matching headings so the hits show under them.
				if (q.trim() && groups.length <= 10)
					for (const g of groups) {
						open[`g:${g.key}`] = true;
						if (g.playlists.length <= 3) for (const s of g.playlists) open[sourceKey(g, s)] = true;
					}
			}
			const jobs: Promise<void>[] = [];
			for (const g of groups)
				if (open[`g:${g.key}`])
					for (const [key, group, policy] of leaves(g))
						if (g.playlists.length === 0 || open[key]) jobs.push(fetchPage(key, group, policy));
			await Promise.all(jobs);
			loaded = true;
		});
	}

	function toggle(key: string, group: string, policy: string | null, leaf: boolean) {
		open[key] = !open[key];
		if (open[key] && leaf && !pages[key]) ps.run(() => fetchPage(key, group, policy));
	}

	function toggleGroup(g: Group) {
		const key = `g:${g.key}`;
		open[key] = !open[key];
		if (open[key] && g.playlists.length === 0 && !pages[key]) ps.run(() => fetchPage(key, g.key, null));
	}

	let timer: ReturnType<typeof setTimeout>;
	function onSearch() {
		clearTimeout(timer);
		timer = setTimeout(() => refresh(true), 200);
	}

	async function act(method: string, path: string) {
		await ps.act(method, path, {});
		await refresh();
	}

	let first = true;
	$effect(() => {
		// A policy added or removed elsewhere on the page reloads the tree.
		void ps.policies;
		untrack(() => refresh(first));
		first = false;
		return () => clearTimeout(timer);
	});
</script>

{#snippet rows(key: string, group: string, policy: string | null)}
	{@const p = pages[key]}
	{#if p}
		<ul class="lib-videos">
			{#each p.videos as v (v.youtube_id + v.policy_id)}
				<li>
					<span class="grow">{v.title || v.youtube_id}</span>
					<span class="stat length" title="Video length">{fmtLength(v.duration_s)}</span>
					<span class="stat" title="Total plays">{v.plays} {v.plays === 1 ? 'play' : 'plays'}</span>
					<span class="stat" title="Total time watched">{fmtWatched(v.watched_s)}</span>
					<span class="actions">
						{#if view === 'review'}
							<ActionButton class="primary" run={() => act('POST', `/api/parent/videos/${v.youtube_id}/approve`)} done="Approved {v.title || v.youtube_id}" fail="approve it">Approve</ActionButton>
						{/if}
						<ActionButton class="danger" run={() => act('POST', `/api/parent/videos/${v.youtube_id}/block`)} done="Blocked {v.title || v.youtube_id}" fail="block it">Block</ActionButton>
					</span>
				</li>
			{/each}
		</ul>
		{#if p.videos.length < p.total}
			<ActionButton class="more" disabled={p.loading} run={() => fetchPage(key, group, policy, true)} fail="load more videos">
				Show more ({p.videos.length} of {p.total})
			</ActionButton>
		{/if}
	{:else}
		<p class="muted">Loading…</p>
	{/if}
{/snippet}

<div class="lib">
	<div class="lib-tools">
		<input type="search" aria-label="Search videos" placeholder="Search title, channel or tag" bind:value={q} oninput={onSearch} />
		<select aria-label="Show plays for" bind:value={child} onchange={() => refresh()}>
			<option value="">All children</option>
			{#each ps.children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
		</select>
		<select aria-label="Sort videos" bind:value={sort} onchange={() => refresh()}>
			<option value="title">Sort by title</option>
			<option value="plays">Most plays</option>
			<option value="time">Most time watched</option>
		</select>
	</div>

	<ul class="lib-groups">
		{#each groups as g (g.key)}
			{@const gk = `g:${g.key}`}
			<li class="group">
				<button class="head" aria-expanded={!!open[gk]} onclick={() => toggleGroup(g)}>
					<span class="caret" aria-hidden="true">{open[gk] ? '▾' : '▸'}</span>
					<span class="grow">{groupTitle(g)}</span>
					<span class="pill">{g.count} {g.count === 1 ? 'video' : 'videos'}</span>
				</button>
				{#if open[gk]}
					<div class="body">
						{#if g.playlists.length === 0}
							{@render rows(gk, g.key, null)}
						{:else}
							{#each g.playlists as s (s.policy_id)}
								{@const sk = sourceKey(g, s)}
								<div class="sub">
									<button class="head" aria-expanded={!!open[sk]} onclick={() => toggle(sk, g.key, s.policy_id, true)}>
										<span class="caret" aria-hidden="true">{open[sk] ? '▾' : '▸'}</span>
										<span class="grow">{sourceTitle(s)}</span>
										<span class="pill">{s.count} {s.count === 1 ? 'video' : 'videos'}</span>
									</button>
									{#if open[sk]}{@render rows(sk, g.key, s.policy_id)}{/if}
								</div>
							{/each}
						{/if}
					</div>
				{/if}
			</li>
		{:else}
			{#if loaded}
				<li class="empty">{q.trim() ? 'No videos match that search.' : view === 'review' ? 'Nothing waiting for review.' : 'No videos are allowed yet.'}</li>
			{/if}
		{/each}
	</ul>
</div>

<style>
	.lib-tools { display: grid; gap: var(--space-2); margin-bottom: var(--space-3); }
	@media (min-width: 640px) { .lib-tools { grid-template-columns: 1fr 12rem 12rem; } }
	ul { list-style: none; margin: 0; padding: 0; }
	.lib-groups { display: grid; gap: var(--space-2); }
	.group { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); overflow: hidden; }
	.empty { color: var(--muted); text-align: center; padding: var(--space-5) var(--space-3); border: 1px dashed var(--border); border-radius: var(--radius); }
	.head { display: flex; align-items: center; gap: var(--space-2); width: 100%; text-align: left; border: 0; border-radius: 0; background: transparent; }
	.grow { flex: 1 1 10rem; min-width: 0; overflow-wrap: anywhere; }
	.caret { width: 1rem; color: var(--muted); }
	.body { border-top: 1px solid var(--border); padding: 0 var(--space-3) var(--space-3); }
	.sub .head { padding-left: var(--space-1); }
	.lib-videos { display: grid; gap: 1px; }
	.lib-videos li { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1) var(--space-3); padding: var(--space-2) 0; border-bottom: 1px solid var(--border); }
	.stat { color: var(--muted); font-size: 0.85rem; font-variant-numeric: tabular-nums; white-space: nowrap; }
	.actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
	.lib :global(.more) { width: 100%; margin-top: var(--space-2); }
	.muted { color: var(--muted); }
</style>
