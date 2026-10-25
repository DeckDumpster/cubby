<script lang="ts">
	import { toasts } from '$lib/toast.svelte.js';
</script>

<div class="toasts" role="region" aria-label="Notifications" aria-live="polite">
	{#each toasts.items as t (t.id)}
		<div class="toast" class:bad={t.kind === 'error'} data-kind={t.kind}>
			<span class="msg">{t.kind === 'ok' ? '✓ ' : ''}{t.text}</span>
			{#if t.kind === 'error'}<button type="button" aria-label="Dismiss notification" onclick={() => toasts.dismiss(t.id)}>✕</button>{/if}
		</div>
	{/each}
</div>

<style>
	.toasts { position: fixed; z-index: 10; left: var(--space-3); right: var(--space-3); bottom: var(--space-3); display: grid; gap: var(--space-2); justify-items: center; pointer-events: none; }
	.toast { pointer-events: auto; box-sizing: border-box; max-width: min(100%, 28rem); display: flex; align-items: center; gap: var(--space-3); background: var(--surface); color: var(--text); border: 1px solid var(--ok); border-left-width: 6px; border-radius: 10px; box-shadow: var(--shadow), 0 6px 20px rgb(0 0 0 / 0.2); padding: var(--space-3) var(--space-4); animation: rise 0.2s ease-out; }
	.toast.bad { border-color: var(--danger); background: var(--danger-bg); color: var(--danger); }
	.msg { min-width: 0; overflow-wrap: anywhere; }
	button { flex: none; font: inherit; color: inherit; background: transparent; border: 0; min-width: var(--touch); min-height: var(--touch); margin: calc(var(--space-2) * -1) calc(var(--space-3) * -1) calc(var(--space-2) * -1) 0; cursor: pointer; }
	@keyframes rise { from { opacity: 0; transform: translateY(0.5rem); } }
	@media (prefers-reduced-motion: reduce) { .toast { animation: none; } }
</style>
