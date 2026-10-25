<script lang="ts">
	import { api, rulesBody } from '$lib/parent.js';
	import { getParent } from '$lib/parentState.svelte.js';
	import type { Policy } from '$lib/parentState.svelte.js';

	let { policy }: { policy: Policy } = $props();
	const ps = getParent();

	type Excluded = { youtube_id: string; title: string; duration_s: number | null };
	type Counts = { allowed: number; excluded: number; excluded_videos: Excluded[] };

	const saved = $derived(policy.rules);
	const text = (n: number | null | undefined) => (n == null ? '' : String(n));
	let form = $state({ exclude_shorts: false, min: '', max: '', has: '', lacks: '' });
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
			has: (saved?.title_has ?? []).join(', '),
			lacks: (saved?.title_lacks ?? []).join(', ')
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

	async function save() {
		const body = rulesBody(form);
		if (!body) return;
		await ps.act('PUT', `/api/parent/policies/${policy.id}/rules`, body);
	}

	const dur = (s: number | null) => (s == null ? '?' : s < 60 ? `${s}s` : `${Math.round(s / 60)} min`);
</script>

<details class="rules" bind:open>
	<summary>Filters <small>— {summary}</small></summary>
	<div class="fields">
		<label class="check"><input type="checkbox" bind:checked={form.exclude_shorts} onchange={changed} /> Exclude Shorts (60 seconds or less)</label>
		<label>Shortest, minutes <input inputmode="decimal" bind:value={form.min} oninput={changed} /></label>
		<label>Longest, minutes <input inputmode="decimal" bind:value={form.max} oninput={changed} /></label>
		<label>Title contains one of <input placeholder="e.g. episode, full" bind:value={form.has} oninput={changed} /></label>
		<label>Title contains none of <input placeholder="e.g. compilation, mix" bind:value={form.lacks} oninput={changed} /></label>
	</div>
	{#if bad}<p class="error" role="alert">Durations are numbers of minutes, and the shortest cannot exceed the longest.</p>{/if}
	{#if counts}
		<p class="count" aria-live="polite"><b>{counts.allowed}</b> allowed · <b>{counts.excluded}</b> excluded (not saved yet)</p>
		{#if counts.excluded_videos.length}
			<details>
				<summary>Excluded videos</summary>
				<ul>
					{#each counts.excluded_videos as v (v.youtube_id)}
						<li><span class="grow">{v.title || v.youtube_id}</span> <small>{dur(v.duration_s)}</small></li>
					{/each}
				</ul>
				{#if counts.excluded > counts.excluded_videos.length}<p class="muted">…and {counts.excluded - counts.excluded_videos.length} more.</p>{/if}
			</details>
		{/if}
	{/if}
	<div class="actions">
		<button class="primary" disabled={bad} onclick={save}>Save filters</button>
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
	.actions { display: flex; gap: var(--space-2); }
	.muted { color: var(--muted); font-size: 0.85rem; }
</style>
