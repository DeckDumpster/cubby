<script lang="ts">
	import { page } from '$app/state';
	import { getParent } from '$lib/parentState.svelte.js';
	import ChildCard from '$lib/ChildCard.svelte';
	const ps = getParent();
	const c = $derived(ps.children.find((x) => x.id === page.params.id));
</script>

<svelte:head><title>{c ? `${c.name} · Cubby` : 'Child · Cubby'}</title></svelte:head>

<section class="panel">
	<p><a href="/parent/children">← All children</a></p>
	{#if c}
		<ChildCard {c} />
	{:else}
		<p class="empty">No such child.</p>
	{/if}
</section>
