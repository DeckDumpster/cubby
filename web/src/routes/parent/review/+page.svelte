<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	const ps = getParent();
</script>

<section class="panel">
	<h2>Added by your policies</h2>
	<select bind:value={ps.reviewChannel} onchange={ps.load}>
		<option value="">All channels</option>
		{#each ps.channels as [id, title] (id)}<option value={id}>{title}</option>{/each}
	</select>
	<ul class="list">
		{#each ps.review as i (i.youtube_id + i.policy_youtube_id)}
			<li>
				<span class="grow">{i.title} <small>({i.channel_title}, via {i.policy_kind} {i.policy_youtube_id})</small></span>
				<span class="actions">
				<button class="primary" onclick={() => ps.act('POST', `/api/parent/videos/${i.youtube_id}/approve`, {})}>Approve</button>
				<button class="danger" onclick={() => ps.act('POST', `/api/parent/videos/${i.youtube_id}/block`, {})}>Block</button>
				</span>
			</li>
		{:else}<li class="empty">Nothing waiting for review.</li>{/each}
	</ul>
</section>
