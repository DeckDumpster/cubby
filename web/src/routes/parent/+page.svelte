<script lang="ts">
	import { onMount } from 'svelte';
	import { api, budgetBody, fmtWatched, requestLabel, windowsToText } from '$lib/parent.js';

	type Device = { id: string; label: string; revoked_at: number | null };
	type Child = {
		id: string;
		name: string;
		avatar: string;
		tz: string;
		devices: Device[];
		budget: {
			video_equivalents: number | null;
			minutes_cap: number | null;
			cutoff_mid_play: boolean;
			daily_windows: [string, string][];
		};
	};
	type Policy = { id: string; kind: string; youtube_id: string; child_id: string | null; title: string };
	type Item = { youtube_id: string; title: string; channel_id: string | null; channel_title: string; policy_kind: string; policy_youtube_id: string };
	type Req = { id: string; child_name: string; kind: string; title: string; channel_title: string; youtube_id: string };
	type Grant = { id: string; title: string; youtube_id: string; granted_at: number; watched_s: number; charge: number; end_reason: string | null };

	let tab = $state('allow');
	let error = $state('');
	let children: Child[] = $state([]);
	let policies: Policy[] = $state([]);
	let review: Item[] = $state([]);
	let requests: Req[] = $state([]);
	let history: Grant[] = $state([]);
	let historyChild = $state('');
	let reviewChannel = $state('');

	let url = $state('');
	let scope = $state('');
	let preview: { kind: string; youtube_id: string } | null = $state(null);
	let pairing: { child: string; code: string } | null = $state(null);
	let newChild = $state({ name: '', avatar: '', tz: Intl.DateTimeFormat().resolvedOptions().timeZone });
	let forms: Record<string, { videos: string; minutes: string; windows: string; cutoff: boolean }> = $state({});

	async function run(fn: () => Promise<unknown>) {
		error = '';
		try {
			await fn();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}

	async function load() {
		await run(async () => {
			children = await api('GET', '/api/parent/children');
			for (const c of children)
				forms[c.id] = {
					videos: c.budget.video_equivalents?.toString() ?? '',
					minutes: c.budget.minutes_cap?.toString() ?? '',
					windows: windowsToText(c.budget.daily_windows),
					cutoff: c.budget.cutoff_mid_play
				};
			policies = await api('GET', '/api/parent/policies');
			review = await api(
				'GET',
				'/api/parent/review' + (reviewChannel ? `?channel=${encodeURIComponent(reviewChannel)}` : '')
			);
			requests = await api('GET', '/api/parent/requests');
			if (!historyChild && children.length) historyChild = children[0].id;
			if (historyChild) history = await api('GET', `/api/parent/children/${historyChild}/history`);
		});
	}
	onMount(load);

	const childName = (id: string | null) => (id ? (children.find((c) => c.id === id)?.name ?? '?') : 'everyone');
	const channels = $derived([...new Map(review.filter((i) => i.channel_id).map((i) => [i.channel_id!, i.channel_title])).entries()]);

	const doPreview = () => run(async () => (preview = await api('POST', '/api/parent/policies/preview', { url })));
	const allow = () =>
		run(async () => {
			await api('POST', '/api/parent/policies', { url, child_id: scope || null });
			url = '';
			preview = null;
			await load();
		});
	const act = (method: string, path: string, body?: unknown) =>
		run(async () => {
			await api(method, path, body);
			await load();
		});
	const saveBudget = (id: string) =>
		run(async () => {
			const body = budgetBody(forms[id]);
			if (!body) throw new Error('Check the budget: numbers, and windows like 07:00-19:00');
			await api('PUT', `/api/parent/children/${id}/budget`, body);
			await load();
		});
	const pair = (id: string) =>
		run(async () => {
			const r = await api('POST', `/api/parent/children/${id}/pairing`);
			pairing = { child: id, code: r.code };
		});
	const addChild = () =>
		run(async () => {
			await api('POST', '/api/parent/children', newChild);
			newChild.name = '';
			await load();
		});
	const when = (t: number) => new Date(t * 1000).toLocaleString();
</script>

<h1>Cubby — parent</h1>
<nav>
	{#each [['allow', 'Allow-list'], ['review', 'Added by your policies'], ['requests', `Requests (${requests.length})`], ['children', 'Children'], ['history', 'History']] as [key, label] (key)}
		<button class:active={tab === key} onclick={() => (tab = key)}>{label}</button>
	{/each}
</nav>
{#if error}<p class="error" role="alert">{error}</p>{/if}

{#if tab === 'allow'}
	<section>
		<h2>Allow a video, channel or playlist</h2>
		<input placeholder="Paste a YouTube link" bind:value={url} oninput={() => (preview = null)} />
		<select bind:value={scope}>
			<option value="">All children</option>
			{#each children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
		</select>
		<button onclick={doPreview} disabled={!url}>Check</button>
		{#if preview}
			<div class="card">
				<b>{preview.kind}</b> <code>{preview.youtube_id}</code> for {childName(scope || null)}
				<button onclick={allow}>Allow</button>
			</div>
		{/if}
		<ul>
			{#each policies as p (p.id)}
				<li>
					<b>{p.kind}</b> {p.title || p.youtube_id} — {childName(p.child_id)}
					<button onclick={() => act('DELETE', `/api/parent/policies/${p.id}`)}>Remove</button>
				</li>
			{:else}<li>Nothing is allowed yet, so children see nothing.</li>{/each}
		</ul>
	</section>
{:else if tab === 'review'}
	<section>
		<h2>Added by your policies</h2>
		<select bind:value={reviewChannel} onchange={load}>
			<option value="">All channels</option>
			{#each channels as [id, title] (id)}<option value={id}>{title}</option>{/each}
		</select>
		<ul>
			{#each review as i (i.youtube_id + i.policy_youtube_id)}
				<li>
					{i.title} <small>({i.channel_title}, via {i.policy_kind} {i.policy_youtube_id})</small>
					<button onclick={() => act('POST', `/api/parent/videos/${i.youtube_id}/approve`, {})}>Approve</button>
					<button onclick={() => act('POST', `/api/parent/videos/${i.youtube_id}/block`, {})}>Block</button>
				</li>
			{:else}<li>Nothing waiting for review.</li>{/each}
		</ul>
	</section>
{:else if tab === 'requests'}
	<section>
		<h2>Requests</h2>
		<ul>
			{#each requests as r (r.id)}
				<li>
					<b>{r.child_name}</b> {requestLabel(r.kind)}: {r.title || r.youtube_id} <small>({r.channel_title})</small>
					<button onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-channel' })}>Allow channel</button>
					<button onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-more', n: 5 })}>Allow 5 more</button>
					<button onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'dismiss' })}>Dismiss</button>
				</li>
			{:else}<li>No requests.</li>{/each}
		</ul>
	</section>
{:else if tab === 'children'}
	<section>
		<h2>Children</h2>
		{#each children as c (c.id)}
			<div class="card">
				<h3>{c.avatar} {c.name} <small>{c.tz}</small></h3>
				{#if forms[c.id]}
					<label>Videos per day <input bind:value={forms[c.id].videos} /></label>
					<label>Minutes per day <input bind:value={forms[c.id].minutes} /></label>
					<label>Windows <input placeholder="07:00-19:00" bind:value={forms[c.id].windows} /></label>
					<label><input type="checkbox" bind:checked={forms[c.id].cutoff} /> Stop a video mid-play when the budget runs out</label>
					<button onclick={() => saveBudget(c.id)}>Save budget</button>
				{/if}
				<h4>Devices</h4>
				<ul>
					{#each c.devices as d (d.id)}
						<li>
							{d.label || 'device'}
							{#if d.revoked_at}(revoked){:else}
								<button onclick={() => act('POST', `/api/parent/devices/${d.id}/revoke`)}>Revoke</button>
							{/if}
						</li>
					{/each}
				</ul>
				<button onclick={() => pair(c.id)}>Pair a device</button>
				{#if pairing?.child === c.id}<p class="code">{pairing.code}</p>{/if}
			</div>
		{/each}
		<div class="card">
			<h3>Add a child</h3>
			<input placeholder="Name" bind:value={newChild.name} />
			<input placeholder="Avatar" bind:value={newChild.avatar} />
			<input placeholder="Time zone" bind:value={newChild.tz} />
			<button onclick={addChild} disabled={!newChild.name}>Add</button>
		</div>
	</section>
{:else}
	<section>
		<h2>History</h2>
		<select bind:value={historyChild} onchange={load}>
			{#each children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
		</select>
		<table>
			<thead><tr><th>When</th><th>Video</th><th>Watched</th><th>Charge</th><th>Ended</th></tr></thead>
			<tbody>
				{#each history as g (g.id)}
					<tr>
						<td>{when(g.granted_at)}</td><td>{g.title || g.youtube_id}</td>
						<td>{fmtWatched(g.watched_s)}</td><td>{g.charge.toFixed(2)}</td><td>{g.end_reason ?? 'playing'}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</section>
{/if}

<style>
	nav button.active { font-weight: bold; }
	.error { color: #b00020; }
	.card { border: 1px solid #ccc; border-radius: 8px; padding: 0.75rem; margin: 0.75rem 0; }
	.code { font-size: 3rem; letter-spacing: 0.3em; font-family: monospace; }
	label { display: block; }
	small { color: #666; }
</style>
