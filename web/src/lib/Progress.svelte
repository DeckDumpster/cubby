<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/parent.js';
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

	const num = (n: number) => String(Math.round(n * 10) / 10);
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
				<h3><span class="avatar" aria-hidden="true">{c?.avatar}</span> {c?.name ?? 'Child'}</h3>
				{#if r.watching}
					<p class="live">▶ Watching now: <b>{r.watching.title || 'a video'}</b> <small>({num(r.watching.minutes)} min in)</small></p>
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
								<b>{num(s.left ?? 0)} left</b>
							{/if}
						</div>
						{#if s.limit !== null}
							<div class="bar" role="progressbar" aria-label="{label} used" aria-valuemin="0" aria-valuemax={s.limit} aria-valuenow={Math.min(s.used, s.limit)}>
								<span class:full={s.reached} style="width: {pct(s)}%"></span>
							</div>
							<small>{num(s.used)} of {num(s.limit)} used{#if s.reached} · limit reached{/if}</small>
						{:else}
							<small>{num(s.used)} used</small>
						{/if}
					</div>
				{/each}
				{#if r.resets_at && (r.videos.limit !== null || r.minutes.limit !== null)}
					<p class="muted">Resets {when(r.resets_at, r.tz)}</p>
				{/if}
			</article>
		{/each}
	</section>
{/if}

<style>
	.progress { display: grid; gap: var(--space-3); margin: var(--space-3) 0; }
	@media (min-width: 640px) { .progress { grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr)); } }
	.pcard { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); box-shadow: var(--shadow); padding: var(--space-3) var(--space-4); min-width: 0; }
	.pcard h3 { margin: 0 0 var(--space-2); font-size: 1.05rem; display: flex; gap: var(--space-2); align-items: center; }
	.pcard p { margin: var(--space-1) 0; overflow-wrap: anywhere; }
	.live { color: var(--accent); }
	.state { color: var(--danger); font-weight: 600; }
	.muted { color: var(--muted); font-size: 0.9rem; }
	.meter { margin-top: var(--space-2); }
	.mhead { display: flex; justify-content: space-between; gap: var(--space-2); }
	.bar { height: 0.7rem; border-radius: 999px; background: var(--surface-2); overflow: hidden; margin: var(--space-1) 0; }
	.bar span { display: block; height: 100%; background: var(--accent); }
	.bar span.full { background: var(--danger); }
	small { color: var(--muted); }
</style>
