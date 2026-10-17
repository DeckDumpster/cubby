/** The two route trees of the app. Each renders a placeholder until its bead lands. */
export const areas = {
	parent: { path: '/parent', title: 'Parent' },
	watch: { path: '/watch', title: 'Watch' }
};

/** @param {keyof typeof areas} key */
export function placeholder(key) {
	return `${areas[key].title} area — coming soon`;
}
