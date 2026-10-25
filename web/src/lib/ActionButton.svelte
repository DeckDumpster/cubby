<script lang="ts">
	import { onDestroy, type Snippet } from 'svelte';
	import { toasts } from '$lib/toast.svelte.js';

	type Props = {
		/** Performs the change; throws when it did not apply. */
		run: () => Promise<unknown>;
		/** The success toast, e.g. "Added 15 min for Lily"; empty for none. A string returned by run wins. */
		done?: string;
		/** What failed, for the error toast: "Could not <fail>: <reason>". */
		fail: string;
		class?: string;
		disabled?: boolean;
		type?: 'button' | 'submit';
		children: Snippet;
		[attr: string]: unknown;
	};
	let { run, done = '', fail, class: cls = '', disabled = false, type = 'button', children, ...rest }: Props = $props();

	let state: 'idle' | 'busy' | 'done' = $state('idle');
	let timer: ReturnType<typeof setTimeout>;
	onDestroy(() => clearTimeout(timer));

	async function press(e: MouseEvent) {
		// A submit button's own click would also submit its form; the form only needs preventDefault.
		if (type === 'submit') e.preventDefault();
		if (state === 'busy') return;
		state = 'busy';
		let result: unknown;
		try {
			result = await run();
		} catch (err) {
			state = 'idle';
			toasts.error(`Could not ${fail}: ${err instanceof Error ? err.message : String(err)}`);
			return;
		}
		state = 'done';
		const text = typeof result === 'string' ? result : done;
		if (text) toasts.ok(text);
		clearTimeout(timer);
		timer = setTimeout(() => (state = 'idle'), 1500);
	}
</script>

<button {...rest} {type} class="{cls} action" class:busy={state === 'busy'} class:done={state === 'done'} disabled={disabled || state === 'busy'} aria-busy={state === 'busy'} onclick={press} data-state={state}>
	{#if state === 'busy'}<span class="spin" aria-hidden="true"></span>{:else if state === 'done'}<span class="tick" aria-hidden="true">✓</span>{/if}
	{@render children()}
</button>

<style>
	.action { transition: transform 0.08s, filter 0.08s; }
	.action:active:not(:disabled) { transform: scale(0.96); filter: brightness(0.85); }
	.action.busy { opacity: 0.75; cursor: progress; }
	.action.done { border-color: var(--ok); color: var(--ok); }
	.action.done.primary { background: var(--ok); color: var(--accent-text); }
	.spin { display: inline-block; width: 0.9em; height: 0.9em; margin-right: 0.4em; vertical-align: -0.1em; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: turn 0.7s linear infinite; }
	.tick { margin-right: 0.3em; }
	@keyframes turn { to { transform: rotate(360deg); } }
	@media (prefers-reduced-motion: reduce) {
		.action { transition: none; }
		.action:active:not(:disabled) { transform: none; }
		.spin { animation: none; border-right-color: currentColor; border-style: dotted; }
	}
</style>
