<script lang="ts">
	import { getParent, AVATARS } from '$lib/parentState.svelte.js';
	import ActionButton from '$lib/ActionButton.svelte';
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
		<div class="card">
			<h3><span class="avatar">{c.avatar}</span> {c.name} <small>{c.tz}</small></h3>
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
						{#if d.revoked_at}<small>(revoked)</small>{:else}
							<ActionButton class="danger" run={() => ps.act('POST', `/api/parent/devices/${d.id}/revoke`)} done="Revoked {d.label || 'the device'}" fail="revoke the device">Revoke</ActionButton>
						{/if}
					</li>
				{/each}
			</ul>
			<ActionButton run={() => ps.pair(c.id)} done="Pairing code ready for {c.name}" fail="make a pairing code">Pair a device</ActionButton>
			{#if ps.pairing?.child === c.id}<p class="code" aria-label="Pairing code">{ps.pairing.code}</p>{/if}
		</div>
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
