<script lang="ts">
	import { api, rulesBody, withPhrase, fmtLength } from '$lib/parent.js';
	import { getParent } from '$lib/parentState.svelte.js';
	import ActionButton from '$lib/ActionButton.svelte';
	import type { Policy } from '$lib/parentState.svelte.js';

	let { policy }: { policy: Policy } = $props();
	const ps = getParent();

	type Excluded = { youtube_id: string; title: string; duration_s: number | null };
	type Counts = { allowed: number; excluded: number; excluded_videos: Excluded[] };

	const saved = $derived(policy.rules);
	const text = (n: number | null | undefined) => (n == null ? '' : String(n));
	let form = $state({ exclude_shorts: false, min: '', max: '', has: [] as string[], lacks: [] as string[], addHas: '', addLacks: '' });
	let counts: Counts | null = $state(null);
	let bad = $state(false);
	let open = $state(false);
	let seq = 0;
	let timer: ReturnType<typeof setTimeout>;

	function reset() {
		form = {
			exclude_shorts: saved?.exclude_shorts ?? false,
			min: text(saved?.min_minutes),
			max: text(saved?.max_minutes),
			has: [...(saved?.title_has ?? [])],
			lacks: [...(saved?.title_lacks ?? [])],
			addHas: '',
			addLacks: ''
		};
		counts = null;
	}
	$effect(() => {
		void policy.rules;
		reset();
	});

	const summary = $derived(
		policy.excluded_count
			? `${policy.allowed_count} allowed · ${policy.excluded_count} excluded`
			: saved && (saved.exclude_shorts || saved.min_minutes != null || saved.max_minutes != null || saved.title_has.length || saved.title_lacks.length)
				? `${policy.allowed_count} allowed · none excluded`
				: 'No filters'
	);

	/** Recounts as the parent types, so the effect of the rules is visible before saving. */
	function changed() {
		clearTimeout(timer);
		const body = rulesBody(form);
		bad = !body;
		if (!body) return;
		const mine = ++seq;
		timer = setTimeout(async () => {
			try {
				const r = await api('POST', `/api/parent/policies/${policy.id}/rules/preview`, body);
				if (mine === seq) counts = r;
			} catch (e) {
				ps.error = e instanceof Error ? e.message : String(e);
			}
		}, 250);
	}

	function add(which: 'has' | 'lacks') {
		const key = which === 'has' ? 'addHas' : 'addLacks';
		form[which] = withPhrase(form[which], form[key]);
		form[key] = '';
		changed();
	}

	function drop(which: 'has' | 'lacks', i: number) {
		form[which] = form[which].filter((_, j) => j !== i);
		changed();
	}

	async function save() {
		const body = rulesBody(form);
		if (!body) throw new Error('Check the filters');
		await ps.act('PUT', `/api/parent/policies/${policy.id}/rules`, body);
	}

</script>

<details class="rules" bind:open>
	<summary>Filters <small>— {summary}</small></summary>
	<div class="fields">
		<label class="check"><input type="checkbox" bind:checked={form.exclude_shorts} onchange={changed} /> Exclude Shorts (60 seconds or less)</label>
		<label>Shortest, minutes <input inputmode="decimal" bind:value={form.min} oninput={changed} /></label>
		<label>Longest, minutes <input inputmode="decimal" bind:value={form.max} oninput={changed} /></label>
		{#each [{ k: 'has', label: 'Title contains any of', add: 'addHas', ph: 'e.g. episode' }, { k: 'lacks', label: 'Title contains none of', add: 'addLacks', ph: 'e.g. how to' }] as g (g.k)}
			{@const which = g.k as 'has' | 'lacks'}
			<fieldset class="phrases">
				<legend>{g.label}</legend>
				{#if form[which].length}
					<ul class="chips">
						{#each form[which] as ph, i (ph)}
							<li class="chip"><span class="grow">{ph}</span> <button type="button" aria-label="Remove {ph}" onclick={() => drop(which, i)}>−</button></li>
						{/each}
					</ul>
				{/if}
				<div class="add">
					<input aria-label="Add a phrase to {g.label}" placeholder={g.ph} bind:value={form[g.add as 'addHas' | 'addLacks']} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(which); } }} />
					<button type="button" aria-label="Add phrase to {g.label}" onclick={() => add(which)}>+</button>
				</div>
				<small class="muted">Matches anywhere in the title, ignoring case. A phrase is matched whole, spaces included.</small>
			</fieldset>
		{/each}
	</div>
	{#if bad}<p class="error" role="alert">Durations are numbers of minutes, and the shortest cannot exceed the longest.</p>{/if}
	{#if !counts && policy.allowed_count != null}
		<p class="count" aria-live="polite"><b>{policy.allowed_count}</b> allowed · <b>{policy.excluded_count ?? 0}</b> filtered out</p>
	{/if}
	{#if counts}
		<p class="count" aria-live="polite"><b>{counts.allowed}</b> allowed · <b>{counts.excluded}</b> excluded (not saved yet)</p>
		{#if counts.excluded_videos.length}
			<details>
				<summary>Excluded videos</summary>
				<ul>
					{#each counts.excluded_videos as v (v.youtube_id)}
						<li><span class="grow">{v.title || v.youtube_id}</span> <small>{fmtLength(v.duration_s)}</small></li>
					{/each}
				</ul>
				{#if counts.excluded > counts.excluded_videos.length}<p class="muted">…and {counts.excluded - counts.excluded_videos.length} more.</p>{/if}
			</details>
		{/if}
	{/if}
	<div class="actions">
		<ActionButton class="primary" disabled={bad} run={save} done="Saved the filters" fail="save the filters">Save filters</ActionButton>
		<button onclick={reset}>Cancel</button>
	</div>
	<p class="muted">Rules apply to every video here and to every new upload. Allowing a single video outright overrides them.</p>
</details>

<style>
	.rules { flex-basis: 100%; }
	.fields { display: grid; gap: var(--space-2); margin: var(--space-2) 0; }
	@media (min-width: 640px) { .fields { grid-template-columns: 1fr 1fr; } }
	label { display: grid; gap: var(--space-1); }
	.check { display: flex; align-items: center; gap: var(--space-2); grid-column: 1 / -1; }
	ul { list-style: none; margin: 0; padding: 0; }
	li { display: flex; gap: var(--space-3); padding: var(--space-1) 0; }
	.grow { flex: 1 1 10rem; min-width: 0; overflow-wrap: anywhere; }
	.phrases { border: 0; padding: 0; margin: 0; display: grid; gap: var(--space-1); min-width: 0; }
	.chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
	.chip { display: flex; align-items: center; gap: var(--space-1); padding: 0 0 0 var(--space-2); border: 1px solid var(--muted); border-radius: 999px; }
	.add { display: flex; gap: var(--space-1); }
	.add input { flex: 1 1 auto; min-width: 0; }
	.actions { display: flex; gap: var(--space-2); }
	.muted { color: var(--muted); font-size: 0.85rem; }
</style>
