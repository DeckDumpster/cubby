<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	import { goto } from '$app/navigation';
	import { fmtWatched } from '$lib/parent.js';
	const ps = getParent();
	const when = (t: number) => new Date(t * 1000).toLocaleString();
</script>

<section class="panel">
	<h2>History</h2>
	<select value={ps.historyChild} onchange={(e) => goto(`?child=${encodeURIComponent(e.currentTarget.value)}`, { replaceState: true, keepFocus: true })}>
		{#each ps.children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
	</select>
	<div class="scroll"><table>
		<thead><tr><th>When</th><th>Video</th><th>Watched</th><th>Charge</th><th>Ended</th></tr></thead>
		<tbody>
			{#each ps.history as g (g.id)}
				<tr>
					<td>{when(g.granted_at)}</td><td>{g.title || g.youtube_id}</td>
					<td>{fmtWatched(g.watched_s)}</td><td>{g.charge.toFixed(2)}</td><td>{g.end_reason ?? 'playing'}</td>
				</tr>
			{/each}
		</tbody>
	</table></div>
</section>
