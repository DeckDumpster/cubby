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
	type Policy = { id: string; kind: string; youtube_id: string; child_id: string | null; title: string; pending?: boolean };
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
	let addError = $state('');
	let adding = $state(false);
	let scope = $state('');
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

	const allow = async () => {
		if (!url.trim() || adding) return;
		adding = true;
		addError = '';
		try {
			await api('POST', '/api/parent/policies', { url, child_id: scope || null });
		} catch (e) {
			addError = e instanceof Error ? e.message : String(e);
			return;
		} finally {
			adding = false;
		}
		url = '';
		await load();
	};
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

<div class="parent">
<header class="bar">
	<h1><span class="logo" aria-hidden="true">C</span> Cubby <small>parent</small></h1>
</header>
<nav class="tabs" aria-label="Sections">
	{#each [['allow', 'Allow-list'], ['review', 'Added by your policies'], ['requests', `Requests (${requests.length})`], ['children', 'Children'], ['history', 'History']] as [key, label] (key)}
		<button class="tab" class:active={tab === key} aria-current={tab === key ? 'page' : undefined} onclick={() => (tab = key)}>{label}</button>
	{/each}
</nav>
{#if error}<p class="error" role="alert">{error}</p>{/if}

{#if tab === 'allow'}
	<section class="panel">
		<h2>Allow a video, channel or playlist</h2>
		<form class="form" onsubmit={(e) => { e.preventDefault(); allow(); }}>
		<input placeholder="Paste a YouTube link" bind:value={url} oninput={() => { addError = ''; }} aria-invalid={addError ? 'true' : undefined} />
		<select bind:value={scope}>
			<option value="">All children</option>
			{#each children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
		</select>
		<button class="primary" type="submit" disabled={!url.trim() || adding}>Add</button>
		</form>
		{#if addError}
			<p class="error" role="alert">{addError} <button onclick={() => (addError = '')}>Dismiss</button></p>
		{/if}
		<ul class="list">
			{#each policies as p (p.id)}
				<li>
					<span class="grow"><b class="pill">{p.kind}</b> {p.title || p.youtube_id}{#if p.pending} <b class="pill">pending</b>{/if} <small>— {childName(p.child_id)}</small></span>
					<button class="danger" onclick={() => act('DELETE', `/api/parent/policies/${p.id}`)}>Remove</button>
				</li>
			{:else}<li class="empty">Nothing is allowed yet, so children see nothing.</li>{/each}
		</ul>
	</section>
{:else if tab === 'review'}
	<section class="panel">
		<h2>Added by your policies</h2>
		<select bind:value={reviewChannel} onchange={load}>
			<option value="">All channels</option>
			{#each channels as [id, title] (id)}<option value={id}>{title}</option>{/each}
		</select>
		<ul class="list">
			{#each review as i (i.youtube_id + i.policy_youtube_id)}
				<li>
					<span class="grow">{i.title} <small>({i.channel_title}, via {i.policy_kind} {i.policy_youtube_id})</small></span>
					<span class="actions">
					<button class="primary" onclick={() => act('POST', `/api/parent/videos/${i.youtube_id}/approve`, {})}>Approve</button>
					<button class="danger" onclick={() => act('POST', `/api/parent/videos/${i.youtube_id}/block`, {})}>Block</button>
					</span>
				</li>
			{:else}<li class="empty">Nothing waiting for review.</li>{/each}
		</ul>
	</section>
{:else if tab === 'requests'}
	<section class="panel">
		<h2>Requests</h2>
		<ul class="list">
			{#each requests as r (r.id)}
				<li>
					<span class="grow"><b>{r.child_name}</b> {requestLabel(r.kind)}: {r.title || r.youtube_id} <small>({r.channel_title})</small></span>
					<span class="actions">
					<button class="primary" onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-channel' })}>Allow channel</button>
					<button onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-more', n: 5 })}>Allow 5 more</button>
					<button class="danger" onclick={() => act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'dismiss' })}>Dismiss</button>
					</span>
				</li>
			{:else}<li class="empty">No requests.</li>{/each}
		</ul>
	</section>
{:else if tab === 'children'}
	<section class="panel">
		<h2>Children</h2>
		{#each children as c (c.id)}
			<div class="card">
				<h3><span class="avatar">{c.avatar}</span> {c.name} <small>{c.tz}</small></h3>
				{#if forms[c.id]}
					<label>Videos per day <input bind:value={forms[c.id].videos} /></label>
					<label>Minutes per day <input bind:value={forms[c.id].minutes} /></label>
					<label>Windows <input placeholder="07:00-19:00" bind:value={forms[c.id].windows} /></label>
					<label class="check"><input type="checkbox" bind:checked={forms[c.id].cutoff} /> Stop a video mid-play when the budget runs out</label>
					<button class="primary" onclick={() => saveBudget(c.id)}>Save budget</button>
				{/if}
				<h4>Devices</h4>
				<ul class="list">
					{#each c.devices as d (d.id)}
						<li>
							<span class="grow">{d.label || 'device'}</span>
							{#if d.revoked_at}<small>(revoked)</small>{:else}
								<button class="danger" onclick={() => act('POST', `/api/parent/devices/${d.id}/revoke`)}>Revoke</button>
							{/if}
						</li>
					{/each}
				</ul>
				<button onclick={() => pair(c.id)}>Pair a device</button>
				{#if pairing?.child === c.id}<p class="code" aria-label="Pairing code">{pairing.code}</p>{/if}
			</div>
		{/each}
		<div class="card">
			<h3>Add a child</h3>
			<input placeholder="Name" bind:value={newChild.name} />
			<input placeholder="Avatar" bind:value={newChild.avatar} />
			<input placeholder="Time zone" bind:value={newChild.tz} />
			<button class="primary" onclick={addChild} disabled={!newChild.name}>Add</button>
		</div>
	</section>
{:else}
	<section class="panel">
		<h2>History</h2>
		<select bind:value={historyChild} onchange={load}>
			{#each children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
		</select>
		<div class="scroll"><table>
			<thead><tr><th>When</th><th>Video</th><th>Watched</th><th>Charge</th><th>Ended</th></tr></thead>
			<tbody>
				{#each history as g (g.id)}
					<tr>
						<td>{when(g.granted_at)}</td><td>{g.title || g.youtube_id}</td>
						<td>{fmtWatched(g.watched_s)}</td><td>{g.charge.toFixed(2)}</td><td>{g.end_reason ?? 'playing'}</td>
					</tr>
				{/each}
			</tbody>
		</table></div>
	</section>
{/if}
</div>

<style>
	.parent { max-width: 56rem; margin: 0 auto; padding: 0 var(--space-4) var(--space-5); }
	.bar { padding: var(--space-4) 0 var(--space-2); }
	h1 { display: flex; align-items: center; gap: var(--space-2); font-size: 1.4rem; margin: 0; letter-spacing: -0.01em; }
	h1 small { font-size: 0.85rem; font-weight: 500; text-transform: uppercase; letter-spacing: 0.08em; }
	.logo { display: inline-grid; place-items: center; width: 2rem; height: 2rem; border-radius: 9px; background: var(--accent); color: var(--accent-text); font-weight: 800; }
	h2 { font-size: 1.15rem; margin: var(--space-4) 0 var(--space-3); }
	h3 { font-size: 1.05rem; margin: 0 0 var(--space-3); display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
	h4 { font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin: var(--space-4) 0 var(--space-2); }
	small { color: var(--muted); }
	code { font-family: var(--mono); font-size: 0.85em; background: var(--surface-2); padding: 0.1em 0.4em; border-radius: 6px; }

	.tabs { display: flex; gap: var(--space-1); overflow-x: auto; border-bottom: 1px solid var(--border); margin: 0 calc(var(--space-4) * -1); padding: 0 var(--space-4); scrollbar-width: none; position: sticky; top: 0; background: var(--bg); z-index: 1; }
	.tab { flex: none; background: none; border: 0; border-bottom: 3px solid transparent; border-radius: 0; color: var(--muted); font-weight: 600; padding: 0 var(--space-3); }
	.tab.active { color: var(--accent); border-bottom-color: var(--accent); }

	button, select, input:not([type='checkbox']) { font: inherit; color: var(--text); min-height: var(--touch); border-radius: 10px; border: 1px solid var(--border); background: var(--surface); padding: 0 var(--space-3); }
	input:not([type='checkbox']), select { width: 100%; }
	input::placeholder { color: var(--muted); }
	button { cursor: pointer; font-weight: 600; background: var(--surface-2); }
	button:disabled { opacity: 0.5; cursor: not-allowed; }
	button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
	button.danger { color: var(--danger); background: transparent; }
	:is(button, select, input):focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
	input[type='checkbox'] { width: 1.4rem; height: 1.4rem; accent-color: var(--accent); flex: none; }

	label { display: grid; gap: var(--space-1); margin-bottom: var(--space-3); font-size: 0.9rem; font-weight: 600; color: var(--muted); }
	label.check { display: flex; align-items: center; gap: var(--space-3); min-height: var(--touch); font-weight: 400; color: var(--text); }
	.form { display: grid; gap: var(--space-2); margin-bottom: var(--space-4); }

	.card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); box-shadow: var(--shadow); padding: var(--space-4); margin: var(--space-3) 0; }
	.card.preview { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: center; justify-content: space-between; }
	.avatar { font-size: 1.5rem; }
	.pill { display: inline-block; font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.05em; padding: 0.1em 0.55em; border-radius: 999px; background: var(--surface-2); color: var(--muted); }

	.list { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-2); }
	.list li { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--space-3); }
	.list li.empty { color: var(--muted); justify-content: center; padding: var(--space-5) var(--space-3); border-style: dashed; background: transparent; text-align: center; }
	.card .list li { background: var(--surface-2); border: 0; }
	.grow { flex: 1 1 14rem; min-width: 0; overflow-wrap: anywhere; }
	.actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }

	.error { background: var(--danger-bg); color: var(--danger); border: 1px solid var(--danger); border-radius: 10px; padding: var(--space-3); margin: var(--space-3) 0; }
	.code { font-size: 2.5rem; letter-spacing: 0.3em; font-family: var(--mono); text-align: center; background: var(--surface-2); border-radius: 10px; padding: var(--space-3); margin: var(--space-3) 0 0; }

	.scroll { overflow-x: auto; margin-top: var(--space-3); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
	table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
	th, td { text-align: left; padding: var(--space-3); border-bottom: 1px solid var(--border); vertical-align: top; }
	td:nth-child(2) { min-width: 10rem; }
	th { font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); white-space: nowrap; }
	tbody tr:last-child td { border-bottom: 0; }

	@media (min-width: 640px) {
		.form { grid-template-columns: 1fr 12rem auto; }
		.parent { padding: 0 var(--space-5) var(--space-5); }
		.tabs { margin: 0 calc(var(--space-5) * -1); padding: 0 var(--space-5); }
	}
</style>
