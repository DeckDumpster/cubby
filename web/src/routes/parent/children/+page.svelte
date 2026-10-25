<script lang="ts">
	import { getParent, AVATARS } from '$lib/parentState.svelte.js';
	import ActionButton from '$lib/ActionButton.svelte';
	import ChildCard from '$lib/ChildCard.svelte';
	const ps = getParent();
</script>

<svelte:head><title>Children · Cubby</title></svelte:head>

<section class="panel">
	<h2>Children</h2>
	<div class="card">
	<h3>When time is up</h3>
	<label>Minutes to keep browsing <input inputmode="decimal" data-testid="grace-minutes" bind:value={ps.graceMinutes} /></label>
	<small>When a child reaches their limit, or their viewing window closes, they can still look around and save videos for this long, but not play. Then the whole child site shows "All done for today" until the next day starts or you give them more time.</small>
	<ActionButton class="primary" run={ps.saveGrace} done="Saved how long a child can keep browsing" fail="save that">Save</ActionButton>
	</div>
	{#each ps.children as c (c.id)}
		<ChildCard {c} />
	{/each}
	<div class="card">
		<h3>Add a child</h3>
		<input placeholder="Name" bind:value={ps.newChild.name} />
		<div class="picker" role="group" aria-label="Picture (shown on her screen)">
			<small>Picture (shown on her screen)</small>
			{#each AVATARS as e (e)}
				<button type="button" aria-label={e} aria-pressed={ps.newChild.avatar === e} onclick={() => (ps.newChild.avatar = e)}>{e}</button>
			{/each}
		</div>
		<input placeholder="Time zone" bind:value={ps.newChild.tz} />
		<ActionButton class="primary" run={ps.addChild} done="Child added" fail="add the child" disabled={!ps.newChild.name}>Add</ActionButton>
	</div>
	<div class="card">
		<h3>Parents</h3>
		<ul class="list" aria-label="Parents">
			{#each ps.parents as email (email)}
				<li>{email}</li>
			{/each}
		</ul>
	</div>
</section>

<style>
	.picker { display: flex; flex-wrap: wrap; gap: 0.25rem; margin: 0.5rem 0; align-items: center; }
	.picker small { flex-basis: 100%; }
	.picker :global(button) { font-size: 1.4rem; min-width: var(--touch); padding: 0; }
	.picker :global(button[aria-pressed='true']) { outline: 3px solid var(--accent); }
</style>
