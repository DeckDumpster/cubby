<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	import { requestLabel } from '$lib/parent.js';
	const ps = getParent();
</script>

<section class="panel">
	<h2>Requests</h2>
	<ul class="list">
		{#each ps.requests as r (r.id)}
			<li>
				<span class="grow"><b>{r.child_name}</b> {requestLabel(r.kind)}: {r.title || r.youtube_id} <small>({r.channel_title})</small></span>
				<span class="actions">
				<button class="primary" onclick={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-channel' })}>Allow channel</button>
				<button onclick={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-more', n: 5 })}>Allow 5 more</button>
				<button class="danger" onclick={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'dismiss' })}>Dismiss</button>
				</span>
			</li>
		{:else}<li class="empty">No requests.</li>{/each}
	</ul>
</section>
