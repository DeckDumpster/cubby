<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/parent.js';
	import ActionButton from '$lib/ActionButton.svelte';
	import { getParent } from '$lib/parentState.svelte.js';

	type Side = { limit: number | null; used: number; left: number | null; reached: boolean };
	type Row = {
		child_id: string;
		tz: string;
		videos: Side;
		minutes: Side;
		resets_at: number | null;
		window: { restricted: boolean; open: boolean; opens_at: number | null; closes_at: number | null };
		watching: { title: string; minutes: number } | null;
		grants: { id: string; at: number; expires_at: number; minutes: number; videos: number; by: string; note: string | null; removed_at: number | null; removed_by: string | null }[];
	};

	const ps = getParent();
	let rows: Row[] = $state([]);

	const POLL_MS = 15000;
	const load = async () => {
		try {
			const r = await api('GET', '/api/parent/progress');
			rows = Array.isArray(r) ? r : [];
		} catch {
			/* the next poll tries again; the section's own load reports real errors */
		}
	};
	onMount(() => {
		load();
		const t = setInterval(() => !document.hidden && load(), POLL_MS);
		const vis = () => !document.hidden && load();
		document.addEventListener('visibilitychange', vis);
		return () => {
			clearInterval(t);
			document.removeEventListener('visibilitychange', vis);
		};
	});

	let custom: Record<string, { minutes: string; videos: string }> = $state({});
	let openLive: Record<string, boolean> = $state({});
	const grant = async (id: string, minutes: number, videos: number) => {
		const c = kid(id);
		if (!(minutes > 0 || videos > 0)) throw new Error('enter minutes or videos first');
		await api('POST', `/api/parent/children/${id}/extra`, { minutes, videos });
		if (custom[id]) custom[id] = { minutes: '', videos: '' };
		await load();
		return extraText((c?.name ?? 'child'), minutes, videos);
	};
	const extraText = (name: string, minutes: number, videos: number) =>
		`Added ${[minutes ? `${num(minutes)} min` : '', videos ? `${num(videos)} video${videos === 1 ? '' : 's'}` : ''].filter(Boolean).join(' and ')} for ${name}`;

	const remove = async (child: string, grantId: string) => {
		await api('DELETE', `/api/parent/children/${child}/extra/${grantId}`);
		await load();
		return 'Removed that extra';
	};

	const num = (n: number) => String(Math.round(n * 10) / 10);
	const fmt = (label: string, n: number) => (label === 'Videos' ? n.toFixed(2) : num(n));
	const pct = (s: Side) => (s.limit ? Math.min(100, (s.used / s.limit) * 100) : 0);
	const at = (t: number, tz: string, day: boolean) =>
		new Intl.DateTimeFormat(undefined, {
			timeZone: tz,
			hour: 'numeric',
			minute: '2-digit',
			...(day ? { weekday: 'short' } : {})
		}).format(new Date(t * 1000));
	const sameDay = (t: number, tz: string) => {
		const d = (x: number) => new Intl.DateTimeFormat('en-CA', { timeZone: tz }).format(new Date(x * 1000));
		return d(t) === d(Date.now() / 1000);
	};
	const when = (t: number, tz: string) => at(t, tz, !sameDay(t, tz));
	const kid = (id: string) => ps.children.find((c) => c.id === id);
</script>

{#if rows.length}
	<section class="progress" aria-label="Progress against limits">
		{#each rows as r (r.child_id)}
			{@const c = kid(r.child_id)}
			<article class="pcard" aria-label="{c?.name ?? 'Child'} progress">
				<h3>
					<span class="name"><span class="avatar" aria-hidden="true">{c?.avatar}</span> {c?.name ?? 'Child'}</span>
					{#if r.watching}
						<button type="button" class="live" aria-expanded={!!openLive[r.child_id]} aria-controls="live-{r.child_id}" onclick={() => (openLive[r.child_id] = !openLive[r.child_id])}>▶ Watching <span aria-hidden="true">{openLive[r.child_id] ? '▴' : '▾'}</span></button>
					{/if}
				</h3>
				{#if r.watching && openLive[r.child_id]}
					<p class="livedetail" id="live-{r.child_id}"><b>{r.watching.title || 'a video'}</b> <small>· {num(r.watching.minutes)} min in</small></p>
				{/if}
				{#if r.window.restricted && !r.window.open}
					<p class="state">Viewing is closed{#if r.window.opens_at} until {when(r.window.opens_at, r.tz)}{/if}.</p>
				{:else if r.window.closes_at}
					<p class="muted">Viewing open until {when(r.window.closes_at, r.tz)}.</p>
				{/if}
				{#each [['Videos', r.videos, 'left'], ['Minutes', r.minutes, 'left']] as const as [label, s] (label)}
					<div class="meter">
						<div class="mhead">
							<span>{label}</span>
							{#if s.limit === null}
								<b>No limit</b>
							{:else}
								<b>{fmt(label, s.left ?? 0)} left</b>
							{/if}
						</div>
						{#if s.limit !== null}
							<div class="track" role="progressbar" aria-label="{label} used" aria-valuemin="0" aria-valuemax={s.limit} aria-valuenow={Math.min(s.used, s.limit)}>
								<span class:full={s.reached} style="width: {pct(s)}%"></span>
							</div>
							<small>{fmt(label, s.used)} of {fmt(label, s.limit)} used{#if s.reached} · limit reached{/if}</small>
						{:else}
							<small>{fmt(label, s.used)} used</small>
						{/if}
					</div>
				{/each}
				<div class="extra" role="group" aria-label="Give {c?.name ?? 'child'} extra for today">
					<ActionButton run={() => grant(r.child_id, 15, 0)} fail="add the extra time">+15 min</ActionButton>
					<ActionButton run={() => grant(r.child_id, 0, 1)} fail="add the extra time">+1 video</ActionButton>
				</div>
				<form class="extra" onsubmit={(ev) => ev.preventDefault()}>
					<input type="number" min="0" step="1" inputmode="numeric" placeholder="min" aria-label="Extra minutes" value={custom[r.child_id]?.minutes ?? ''} oninput={(ev) => (custom[r.child_id] = { ...custom[r.child_id], videos: custom[r.child_id]?.videos ?? '', minutes: ev.currentTarget.value })} />
					<input type="number" min="0" step="0.5" inputmode="decimal" placeholder="videos" aria-label="Extra videos" value={custom[r.child_id]?.videos ?? ''} oninput={(ev) => (custom[r.child_id] = { ...custom[r.child_id], minutes: custom[r.child_id]?.minutes ?? '', videos: ev.currentTarget.value })} />
					<ActionButton type="submit" run={() => grant(r.child_id, Number(custom[r.child_id]?.minutes) || 0, Number(custom[r.child_id]?.videos) || 0)} fail="add the extra time">Add</ActionButton>
				</form>
				{#if r.grants?.length}
					<ul class="grants" aria-label="Extra given today">
						{#each r.grants as g (g.id)}
							<li class:removed={g.removed_at !== null}>
								<span class:struck={g.removed_at !== null}>+{[g.minutes ? `${num(g.minutes)} min` : '', g.videos ? `${num(g.videos)} video${g.videos === 1 ? '' : 's'}` : ''].filter(Boolean).join(' and ')}</span>
								<small>· {g.by} · {when(g.at, r.tz)}{#if g.note} · {g.note}{/if}</small>
								{#if g.removed_at !== null}
									<small>· removed by {g.removed_by} at {when(g.removed_at, r.tz)}</small>
								{:else}
									<ActionButton class="remove" run={() => remove(r.child_id, g.id)} fail="remove that extra">(Remove)</ActionButton>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</article>
		{/each}
	</section>
{/if}

<style>
	.progress { display: grid; gap: var(--space-3); margin: var(--space-3) 0; }
	@media (min-width: 640px) { .progress { grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr)); } }
	.pcard { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); box-shadow: var(--shadow); padding: var(--space-3) var(--space-4); min-width: 0; }
	.pcard h3 { margin: 0 0 var(--space-2); font-size: 1.05rem; display: flex; gap: var(--space-2); align-items: center; min-height: 2.75rem; }
	.name { flex: 1 1 auto; min-width: 0; overflow-wrap: anywhere; }
	.pcard p { margin: var(--space-1) 0; overflow-wrap: anywhere; }
	.live { flex: none; margin-left: auto; min-height: 2.75rem; padding: 0 var(--space-2); background: none; border: 0; font: inherit; font-size: 0.9rem; font-weight: 600; color: var(--accent); cursor: pointer; white-space: nowrap; }
	.livedetail { color: var(--accent); }
	.state { color: var(--danger); font-weight: 600; }
	.muted { color: var(--muted); font-size: 0.9rem; }
	.meter { margin-top: var(--space-2); }
	.mhead { display: flex; justify-content: space-between; gap: var(--space-2); }
	.track { height: 0.7rem; border-radius: 999px; background: var(--surface-2); overflow: hidden; margin: var(--space-1) 0; }
	.track span { display: block; height: 100%; background: var(--accent); }
	.track span.full { background: var(--danger); }
	.extra { display: flex; flex-wrap: wrap; gap: var(--space-2); margin: var(--space-2) 0; }
	.extra :global(button), .extra input { min-height: 2.75rem; font: inherit; }
	.extra :global(button) { flex: 1 1 6rem; }
	.extra input { flex: 1 1 4rem; min-width: 0; width: 4rem; }
	.grants { margin: var(--space-2) 0 0; padding-left: var(--space-4); }
	.struck { text-decoration: line-through; }
	.grants li { margin-bottom: var(--space-1); }
	.grants :global(.remove) { min-height: 2.75rem; min-width: 2.75rem; font: inherit; background: none; border: 0; color: var(--danger); text-decoration: underline; }
	small { color: var(--muted); }
</style>
