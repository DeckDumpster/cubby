import { redirect } from '@sveltejs/kit';

// The old child URL: bookmarks keep working.
export function load({ url }) {
	redirect(308, '/' + url.search);
}
