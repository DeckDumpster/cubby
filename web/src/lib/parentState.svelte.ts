import { getContext, setContext } from 'svelte';
import { api, budgetBody, windowsToText } from '$lib/parent.js';

export const AVATARS = ['🦊', '🐻', '🐼', '🐯', '🦁', '🐸', '🐙', '🦄', '🐢', '🐶', '🐱', '🚀'];

export type Device = { id: string; label: string; revoked_at: number | null };
export type Child = {
	id: string;
	name: string;
	avatar: string;
	tz: string;
	devices: Device[];
	budget: {
		video_equivalents: number | null;
		minutes_cap: number | null;
		ve_minutes: number;
		cutoff_mid_play: boolean;
		daily_windows: [string, string][];
	};
};
export type Rules = {
	exclude_shorts: boolean;
	min_minutes: number | null;
	max_minutes: number | null;
	title_has: string[];
	title_lacks: string[];
};
export type Policy = {
	id: string;
	kind: string;
	youtube_id: string;
	child_id: string | null;
	title: string;
	pending?: boolean;
	rules?: Rules;
	allowed_count?: number;
	excluded_count?: number;
};
export type Req = { id: string; child_name: string; kind: string; title: string; channel_title: string; youtube_id: string };
export type Grant = { id: string; title: string; youtube_id: string; granted_at: number; watched_s: number; charge: number; end_reason: string | null; thumb?: string | null };

/** Everything the /parent sections share; the layout owns one and the section routes read it. */
export class ParentState {
	error = $state('');
	children: Child[] = $state([]);
	policies: Policy[] = $state([]);
	requests: Req[] = $state([]);
	parents: string[] = $state([]);
	history: Grant[] = $state([]);
	/** The child chosen in the URL (?child=<id>); empty means the first child. */
	historyChildParam = $state('');

	url = $state('');
	addError = $state('');
	adding = $state(false);
	scope = $state('');
	pairing: { child: string; code: string } | null = $state(null);
	newChild = $state({ name: '', avatar: '', tz: Intl.DateTimeFormat().resolvedOptions().timeZone });
	forms: Record<string, { videos: string; veMinutes: string; minutes: string; windows: string; cutoff: boolean }> = $state({});

	historyChild = $derived(this.historyChildParam || this.children[0]?.id || '');

	run = async (fn: () => Promise<unknown>) => {
		this.error = '';
		try {
			await fn();
		} catch (e) {
			this.error = e instanceof Error ? e.message : String(e);
		}
	};

	load = () =>
		this.run(async () => {
			this.children = await api('GET', '/api/parent/children');
			for (const c of this.children)
				this.forms[c.id] = {
					videos: c.budget.video_equivalents?.toString() ?? '',
					veMinutes: c.budget.ve_minutes?.toString() ?? '15',
					minutes: c.budget.minutes_cap?.toString() ?? '',
					windows: windowsToText(c.budget.daily_windows),
					cutoff: c.budget.cutoff_mid_play
				};
			this.policies = await api('GET', '/api/parent/policies');
			this.requests = await api('GET', '/api/parent/requests');
			const parents = await api('GET', '/api/parent/parents');
			this.parents = Array.isArray(parents) ? parents : [];
			await this.loadHistory();
		});

	loadHistory = async () => {
		const id = this.historyChild;
		if (id) this.history = await api('GET', `/api/parent/children/${id}/history`);
	};

	childName = (id: string | null) => (id ? (this.children.find((c) => c.id === id)?.name ?? '?') : 'everyone');

	allow = async () => {
		if (!this.url.trim() || this.adding) return;
		this.adding = true;
		this.addError = '';
		try {
			await api('POST', '/api/parent/policies', { url: this.url, child_id: this.scope || null });
		} catch (e) {
			this.addError = e instanceof Error ? e.message : String(e);
			throw e;
		} finally {
			this.adding = false;
		}
		this.url = '';
		await this.load();
	};
	// The actions below throw on failure: ActionButton turns that into the error toast.
	act = async (method: string, path: string, body?: unknown) => {
		await api(method, path, body);
		await this.load();
	};
	saveBudget = async (id: string) => {
		const body = budgetBody(this.forms[id]);
		if (!body) throw new Error('Check the budget: numbers, and windows like 07:00-19:00');
		await api('PUT', `/api/parent/children/${id}/budget`, body);
		await this.load();
	};
	pair = async (id: string) => {
		const r = await api('POST', `/api/parent/children/${id}/pairing`);
		this.pairing = { child: id, code: r.code };
	};
	setAvatar = async (c: Child, avatar: string) => {
		await api('PUT', `/api/parent/children/${c.id}`, { name: c.name, avatar, tz: c.tz });
		await this.load();
	};
	addChild = async () => {
		await api('POST', '/api/parent/children', this.newChild);
		this.newChild.name = '';
		this.newChild.avatar = '';
		await this.load();
	};
}

const KEY = Symbol('parent');
export const setParent = (s: ParentState) => setContext(KEY, s);
export const getParent = () => getContext<ParentState>(KEY);
