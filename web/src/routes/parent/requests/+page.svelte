<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	import { requestLabel } from '$lib/parent.js';
	import ActionButton from '$lib/ActionButton.svelte';
	const ps = getParent();
</script>

<svelte:head><title>Requests · Cubby</title></svelte:head>

<section class="panel">
	<h2>Requests</h2>
	<ul class="list">
		{#each ps.requests as r (r.id)}
			<li>
				<span class="grow"><b>{r.child_name}</b> {requestLabel(r.kind)}: {r.title || r.youtube_id} <small>({r.channel_title})</small></span>
				<span class="actions">
				<ActionButton class="primary" run={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-channel' })} done="Allowed {r.channel_title || 'the channel'}" fail="allow the channel">Allow channel</ActionButton>
				<ActionButton run={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'allow-more', n: 5 })} done="Allowed 5 more for {r.child_name}" fail="allow 5 more">Allow 5 more</ActionButton>
				<ActionButton class="danger" run={() => ps.act('POST', `/api/parent/requests/${r.id}/resolve`, { action: 'dismiss' })} done="Dismissed the request from {r.child_name}" fail="dismiss the request">Dismiss</ActionButton>
				</span>
			</li>
		{:else}<li class="empty">No requests.</li>{/each}
	</ul>
</section>
