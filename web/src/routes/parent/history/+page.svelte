<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	import { goto } from '$app/navigation';
	import { fmtWatched, dayGroups, fmtTime, endLabel, watchUrl } from '$lib/parent.js';
	const ps = getParent();
	const tz = $derived(ps.children.find((c) => c.id === ps.historyChild)?.tz);
	const groups = $derived(dayGroups(ps.history, tz, Date.now() / 1000));
</script>

<svelte:head><title>History · Cubby</title></svelte:head>

<section class="panel">
	<h2>History</h2>
	<select value={ps.historyChild} onchange={(e) => goto(`?child=${encodeURIComponent(e.currentTarget.value)}`, { replaceState: true, keepFocus: true })}>
		{#each ps.children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
	</select>
	{#each groups as day (day.key)}
		<section class="day" data-testid="day">
			<h3>{day.label}</h3>
			<ul class="list">
				{#each day.items as g (g.id)}
					<li data-testid="play">
						<time class="at">{fmtTime(g.granted_at, tz)}</time>
						{#if g.thumb}<img class="hthumb" src={g.thumb} alt="" />{:else}<span class="hthumb blank"></span>{/if}
						<span class="grow">
							<a href={watchUrl(g.youtube_id)} target="_blank" rel="noopener">{g.title || g.youtube_id}</a>
							<small>Watched {fmtWatched(g.watched_s)} · charge {g.charge.toFixed(2)} · {endLabel(g.end_reason)}</small>
						</span>
					</li>
				{/each}
			</ul>
		</section>
	{:else}
		<p class="empty-note"><small>Nothing played yet.</small></p>
	{/each}
</section>

<style>
	.day h3 { margin: var(--space-4) 0 var(--space-2); }
	.at { flex: none; width: 4.5rem; font-variant-numeric: tabular-nums; color: var(--muted); font-size: 0.9rem; }
	.hthumb { flex: none; width: 5.5rem; aspect-ratio: 16 / 9; object-fit: cover; border-radius: 6px; background: var(--surface-2); }
	.grow { display: grid; gap: 2px; }
	.grow a { color: var(--accent); font-weight: 600; }
</style>
