<script lang="ts">
	import { getParent, AVATARS } from '$lib/parentState.svelte.js';
	import ActionButton from '$lib/ActionButton.svelte';
	let { c } = $props();
	const ps = getParent();
</script>

	<div class="card">
		<h3><span class="avatar">{c.avatar}</span> <a href="/parent/children/{c.id}">{c.name}</a> <small>{c.tz}</small></h3>
		<div class="picker" role="group" aria-label="Picture for {c.name}">
			{#each AVATARS as e (e)}
				<ActionButton aria-label={e} aria-pressed={c.avatar === e} run={() => ps.setAvatar(c, e)} done="Picture changed for {c.name}" fail="change the picture">{e}</ActionButton>
			{/each}
		</div>
		{#if ps.forms[c.id]}
			<label>Video equivalents per day <input bind:value={ps.forms[c.id].videos} /></label>
			<label>Minutes per video equivalent <input bind:value={ps.forms[c.id].veMinutes} /></label>
			<small>A video equivalent is this many minutes of watching, so a long video costs more than a short one. Only time actually watched counts: a 120-minute video left after 10 minutes costs 10 minutes' worth.</small>
			<label>Minutes per day <input bind:value={ps.forms[c.id].minutes} /></label>
			<label>Windows <input placeholder="07:00-19:00" bind:value={ps.forms[c.id].windows} /></label>
			<label class="check"><input type="checkbox" bind:checked={ps.forms[c.id].cutoff} /> Stop a video mid-play when the budget runs out</label>
			<ActionButton class="primary" run={() => ps.saveBudget(c.id)} done="Saved the budget for {c.name}" fail="save the budget">Save budget</ActionButton>
		{/if}
		<h4>Devices</h4>
		<ul class="list">
			{#each c.devices as d (d.id)}
				<li>
					<span class="grow">{d.label || 'device'}</span>
					<ActionButton class="danger" run={() => ps.act('POST', `/api/parent/devices/${d.id}/revoke`)} done="Revoked {d.label || 'the device'}" fail="revoke the device">Revoke</ActionButton>
				</li>
			{/each}
		</ul>
		<ActionButton run={() => ps.pair(c.id)} done="Pairing code ready for {c.name}" fail="make a pairing code">Pair a device</ActionButton>
		{#if ps.pairing && ps.pairing.child === c.id}<p class="code" aria-label="Pairing code">{ps.pairing.code}</p>{/if}
	</div>

<style>
	.picker { display: flex; flex-wrap: wrap; gap: 0.25rem; margin: 0.5rem 0; align-items: center; }
	.picker small { flex-basis: 100%; }
	.picker :global(button) { font-size: 1.4rem; min-width: var(--touch); padding: 0; }
	.picker :global(button[aria-pressed='true']) { outline: 3px solid var(--accent); }
</style>
