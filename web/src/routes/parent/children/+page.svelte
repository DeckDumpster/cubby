<script lang="ts">
	import { getParent, AVATARS } from '$lib/parentState.svelte.js';
	const ps = getParent();
</script>

<svelte:head><title>Children · Cubby</title></svelte:head>

<section class="panel">
	<h2>Children</h2>
	{#each ps.children as c (c.id)}
		<div class="card">
			<h3><span class="avatar">{c.avatar}</span> {c.name} <small>{c.tz}</small></h3>
			<div class="picker" role="group" aria-label="Picture for {c.name}">
				{#each AVATARS as e (e)}
					<button type="button" aria-label={e} aria-pressed={c.avatar === e} onclick={() => ps.setAvatar(c, e)}>{e}</button>
				{/each}
			</div>
			{#if ps.forms[c.id]}
				<label>Video equivalents per day <input bind:value={ps.forms[c.id].videos} /></label>
				<label>Minutes per video equivalent <input bind:value={ps.forms[c.id].veMinutes} /></label>
				<small>A video equivalent is this many minutes of watching, so a long video costs more than a short one. Only time actually watched counts: a 120-minute video left after 10 minutes costs 10 minutes' worth.</small>
				<label>Minutes per day <input bind:value={ps.forms[c.id].minutes} /></label>
				<label>Windows <input placeholder="07:00-19:00" bind:value={ps.forms[c.id].windows} /></label>
				<label class="check"><input type="checkbox" bind:checked={ps.forms[c.id].cutoff} /> Stop a video mid-play when the budget runs out</label>
				<button class="primary" onclick={() => ps.saveBudget(c.id)}>Save budget</button>
			{/if}
			<h4>Devices</h4>
			<ul class="list">
				{#each c.devices as d (d.id)}
					<li>
						<span class="grow">{d.label || 'device'}</span>
						{#if d.revoked_at}<small>(revoked)</small>{:else}
							<button class="danger" onclick={() => ps.act('POST', `/api/parent/devices/${d.id}/revoke`)}>Revoke</button>
						{/if}
					</li>
				{/each}
			</ul>
			<button onclick={() => ps.pair(c.id)}>Pair a device</button>
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
		<button class="primary" onclick={ps.addChild} disabled={!ps.newChild.name}>Add</button>
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
	.picker button { font-size: 1.4rem; min-width: var(--touch); padding: 0; }
	.picker button[aria-pressed='true'] { outline: 3px solid var(--accent); }
</style>
