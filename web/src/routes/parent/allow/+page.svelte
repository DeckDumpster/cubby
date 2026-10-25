<script lang="ts">
	import PolicyRules from '$lib/PolicyRules.svelte';
	import Library from '$lib/Library.svelte';
	import { getParent } from '$lib/parentState.svelte.js';

	const ps = getParent();
</script>

<svelte:head><title>Allow · Cubby</title></svelte:head>

<section class="panel">
	<h2>Allow a video, channel or playlist</h2>
	<form class="form" onsubmit={(e) => { e.preventDefault(); ps.allow(); }}>
	<input placeholder="Paste a YouTube link" bind:value={ps.url} oninput={() => { ps.addError = ''; }} aria-invalid={ps.addError ? 'true' : undefined} />
	<select bind:value={ps.scope}>
		<option value="">All children</option>
		{#each ps.children as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
	</select>
	<button class="primary" type="submit" disabled={!ps.url.trim() || ps.adding}>Add</button>
	</form>
	{#if ps.addError}
		<p class="error" role="alert">{ps.addError} <button onclick={() => (ps.addError = '')}>Dismiss</button></p>
	{/if}
	<ul class="list">
		{#each ps.policies as p (p.id)}
			<li>
				<span class="grow"><b class="pill">{p.kind}</b> {p.title || p.youtube_id}{#if p.pending} <b class="pill">pending</b>{/if} <small>— {ps.childName(p.child_id)}</small></span>
				<button class="danger" onclick={() => ps.act('DELETE', `/api/parent/policies/${p.id}`)}>Remove</button>
				{#if p.kind !== 'video'}<PolicyRules policy={p} />{/if}
			</li>
		{:else}<li class="empty">Nothing is allowed yet, so children see nothing.</li>{/each}
	</ul>
	<h3>Allowed videos</h3>
	<Library view="allow" />
</section>
