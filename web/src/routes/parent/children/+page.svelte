<script lang="ts">
	import { getParent } from '$lib/parentState.svelte.js';
	const ps = getParent();
</script>

<section class="panel">
	<h2>Children</h2>
	{#each ps.children as c (c.id)}
		<div class="card">
			<h3><span class="avatar">{c.avatar}</span> {c.name} <small>{c.tz}</small></h3>
			{#if ps.forms[c.id]}
				<label>Videos per day <input bind:value={ps.forms[c.id].videos} /></label>
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
		<input placeholder="Avatar" bind:value={ps.newChild.avatar} />
		<input placeholder="Time zone" bind:value={ps.newChild.tz} />
		<button class="primary" onclick={ps.addChild} disabled={!ps.newChild.name}>Add</button>
	</div>
</section>
