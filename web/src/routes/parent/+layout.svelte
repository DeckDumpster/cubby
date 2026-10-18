<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { ParentState, setParent } from '$lib/parentState.svelte.js';

	let { children } = $props();
	const ps = new ParentState();
	setParent(ps);
	onMount(ps.load);

	$effect(() => {
		const id = page.url.searchParams.get('child') ?? '';
		if (id !== ps.historyChildParam) ps.historyChildParam = id;
	});
	let lastHistory = '';
	$effect(() => {
		const id = ps.historyChild;
		if (id && id !== lastHistory) {
			lastHistory = id;
			ps.run(ps.loadHistory);
		}
	});

	const sections = $derived([
		['allow', 'Allow-list'],
		['review', 'Added by your policies'],
		['requests', `Requests (${ps.requests.length})`],
		['children', 'Children'],
		['history', 'History']
	]);
</script>

<div class="parent">
<header class="bar">
	<h1><span class="logo" aria-hidden="true">C</span> Cubby <small>parent</small></h1>
	<a class="signout" href="/cdn-cgi/access/logout">Sign out</a>
</header>
<nav class="tabs" aria-label="Sections">
	{#each sections as [key, label] (key)}
		{@const href = `/parent/${key}`}
		<a class="tab" {href} aria-current={page.url.pathname === href ? 'page' : undefined}>{label}</a>
	{/each}
</nav>
{#if ps.error}<p class="error" role="alert">{ps.error}</p>{/if}

{@render children()}
</div>

<style>
	.parent { max-width: 56rem; margin: 0 auto; padding: 0 var(--space-4) var(--space-5); }
	@media (min-width: 640px) { .parent { padding: 0 var(--space-5) var(--space-5); } }
	.parent :global {

		.bar { display: flex; align-items: center; justify-content: space-between; padding: var(--space-4) 0 var(--space-2); }
		.signout { color: var(--muted); font-size: 0.9rem; }
		h1 { display: flex; align-items: center; gap: var(--space-2); font-size: 1.4rem; margin: 0; letter-spacing: -0.01em; }
		h1 small { font-size: 0.85rem; font-weight: 500; text-transform: uppercase; letter-spacing: 0.08em; }
		.logo { display: inline-grid; place-items: center; width: 2rem; height: 2rem; border-radius: 9px; background: var(--accent); color: var(--accent-text); font-weight: 800; }
		h2 { font-size: 1.15rem; margin: var(--space-4) 0 var(--space-3); }
		h3 { font-size: 1.05rem; margin: 0 0 var(--space-3); display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
		h4 { font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin: var(--space-4) 0 var(--space-2); }
		small { color: var(--muted); }
		code { font-family: var(--mono); font-size: 0.85em; background: var(--surface-2); padding: 0.1em 0.4em; border-radius: 6px; }

		.tabs { display: flex; gap: var(--space-1); overflow-x: auto; border-bottom: 1px solid var(--border); margin: 0 calc(var(--space-4) * -1); padding: 0 var(--space-4); scrollbar-width: none; position: sticky; top: 0; background: var(--bg); z-index: 1; }
		.tab { flex: none; display: inline-flex; align-items: center; text-decoration: none; min-height: var(--touch); background: none; border: 0; border-bottom: 3px solid transparent; border-radius: 0; color: var(--muted); font-weight: 600; padding: 0 var(--space-3); }
		.tab[aria-current='page'] { color: var(--accent); border-bottom-color: var(--accent); }

		button, select, input:not([type='checkbox']) { font: inherit; color: var(--text); min-height: var(--touch); border-radius: 10px; border: 1px solid var(--border); background: var(--surface); padding: 0 var(--space-3); }
		input:not([type='checkbox']), select { width: 100%; }
		input::placeholder { color: var(--muted); }
		button { cursor: pointer; font-weight: 600; background: var(--surface-2); }
		button:disabled { opacity: 0.5; cursor: not-allowed; }
		button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
		button.danger { color: var(--danger); background: transparent; }
		:is(button, select, input):focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
		input[type='checkbox'] { width: 1.4rem; height: 1.4rem; accent-color: var(--accent); flex: none; }

		label { display: grid; gap: var(--space-1); margin-bottom: var(--space-3); font-size: 0.9rem; font-weight: 600; color: var(--muted); }
		label.check { display: flex; align-items: center; gap: var(--space-3); min-height: var(--touch); font-weight: 400; color: var(--text); }
		.form { display: grid; gap: var(--space-2); margin-bottom: var(--space-4); }

		.card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); box-shadow: var(--shadow); padding: var(--space-4); margin: var(--space-3) 0; }
		.card.preview { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: center; justify-content: space-between; }
		.avatar { font-size: 1.5rem; }
		.pill { display: inline-block; font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.05em; padding: 0.1em 0.55em; border-radius: 999px; background: var(--surface-2); color: var(--muted); }

		.list { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-2); }
		.list + * { margin-top: var(--space-3); }
		.list li { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--space-3); }
		.list li.empty { color: var(--muted); justify-content: center; padding: var(--space-5) var(--space-3); border-style: dashed; background: transparent; text-align: center; }
		.card .list li { background: var(--surface-2); border: 0; }
		.grow { flex: 1 1 14rem; min-width: 0; overflow-wrap: anywhere; }
		.actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }

		.error { background: var(--danger-bg); color: var(--danger); border: 1px solid var(--danger); border-radius: 10px; padding: var(--space-3); margin: var(--space-3) 0; }
		.code { font-size: 2.5rem; letter-spacing: 0.3em; font-family: var(--mono); text-align: center; background: var(--surface-2); border-radius: 10px; padding: var(--space-3); margin: var(--space-3) 0 0; }

		.scroll { overflow-x: auto; margin-top: var(--space-3); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
		table { width: 100%; border-collapse: collapse; font-size: 0.9rem; }
		th, td { text-align: left; padding: var(--space-3); border-bottom: 1px solid var(--border); vertical-align: top; }
		td:nth-child(2) { min-width: 10rem; }
		th { font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); white-space: nowrap; }
		tbody tr:last-child td { border-bottom: 0; }

		@media (min-width: 640px) {
			.form { grid-template-columns: 1fr 12rem auto; }
			.tabs { margin: 0 calc(var(--space-5) * -1); padding: 0 var(--space-5); }
		}
	}
</style>
