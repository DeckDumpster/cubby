export type Toast = { id: number; kind: 'ok' | 'error'; text: string };

const OK_MS = 4000;

/** The one toast list for the parent site; Toasts.svelte renders it. Errors stay until dismissed. */
class Toasts {
	items: Toast[] = $state([]);
	#next = 1;

	#add(kind: Toast['kind'], text: string) {
		const id = this.#next++;
		this.items.push({ id, kind, text });
		if (kind === 'ok') setTimeout(() => this.dismiss(id), OK_MS);
	}
	ok = (text: string) => this.#add('ok', text);
	error = (text: string) => this.#add('error', text);
	dismiss = (id: number) => {
		this.items = this.items.filter((t) => t.id !== id);
	};
}

export const toasts = new Toasts();
