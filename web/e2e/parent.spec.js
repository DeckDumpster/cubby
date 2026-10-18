import { test, expect } from '@playwright/test';

const CHILDREN = [
	{ id: 'c1', name: 'Maya', avatar: '🦊', tz: 'America/Chicago', devices: [{ id: 'd1', label: 'Maya iPad', revoked_at: null }, { id: 'd2', label: 'Old phone', revoked_at: 1 }],
		budget: { video_equivalents: 5, minutes_cap: 45, cutoff_mid_play: true, daily_windows: [['07:00', '19:00']] } },
	{ id: 'c2', name: 'Theo', avatar: '🐻', tz: 'America/Chicago', devices: [], budget: { video_equivalents: 3, minutes_cap: null, cutoff_mid_play: false, daily_windows: [] } }
];
const POLICIES = [
	{ id: 'p1', kind: 'channel', youtube_id: 'UC123', child_id: null, title: 'Science for Kids with a rather long channel title' },
	{ id: 'p2', kind: 'video', youtube_id: 'abc', child_id: 'c1', title: 'How volcanoes work' }
];
const REVIEW = [{ youtube_id: 'v1', title: 'Volcano experiments at home', channel_id: 'UC123', channel_title: 'Science for Kids', policy_kind: 'channel', policy_youtube_id: 'UC123' }];
const REQUESTS = [{ id: 'r1', child_name: 'Maya', kind: 'more', title: 'Slime recipes', channel_title: 'Crafty', youtube_id: 'v9' }];
const HISTORY = [{ id: 'g1', title: 'How volcanoes work', youtube_id: 'abc', granted_at: 1760000000, watched_s: 312, charge: 1, end_reason: 'ended' }];

const TABS = ['Allow-list', 'Added by your policies', 'Requests', 'Children', 'History'];
const shots = process.env.SHOTS_DIR;

for (const scheme of ['light', 'dark'])
	for (const [w, h] of [[390, 844], [1280, 800]])
		test(`parent ${scheme} ${w}px`, async ({ page }) => {
			await page.emulateMedia({ colorScheme: scheme });
			await page.setViewportSize({ width: w, height: h });
			await page.route('**/api/parent/**', (route) => {
				const u = route.request().url();
				const body = u.includes('/history') ? HISTORY : u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES
					: u.includes('/review') ? REVIEW : u.includes('/requests') ? REQUESTS : {};
				return route.fulfill({ json: body });
			});
			await page.goto('/parent');
			for (const t of TABS) {
				await page.getByRole('button', { name: new RegExp('^' + t) }).click();
				await expect(page.locator('section.panel')).toBeVisible();
				const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
				expect(overflow, `${t} overflows horizontally`).toBe(false);
				if (shots) await page.screenshot({ path: `${shots}/${scheme}-${w}-${t.split(' ')[0].replace('-', '')}.png`, fullPage: true });
			}
		});

test('a failed add keeps the typed link and shows the error beside it', async ({ page }) => {
	const MSG = 'that is not a YouTube video, channel or playlist link';
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		if (route.request().method() === 'POST' && u.includes('/policies'))
			return route.fulfill({ status: 400, json: { error: MSG } });
		return route.fulfill({ json: u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES : [] });
	});
	await page.goto('/parent');
	const input = page.getByPlaceholder('Paste a YouTube link');
	await input.fill('https://example.com/nope');
	await page.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(page.getByRole('alert')).toContainText(MSG);
	await expect(input).toHaveValue('https://example.com/nope');
	await input.fill('https://example.com/nope2');
	await expect(page.getByRole('alert')).toHaveCount(0);
});

test('one press of Add (or Enter) saves the link and it survives a reload', async ({ page }) => {
	let saved = [...POLICIES];
	const posts = [];
	await page.route('**/api/parent/**', (route) => {
		const r = route.request();
		const u = r.url();
		if (r.method() === 'POST' && u.endsWith('/policies')) {
			posts.push(r.postDataJSON());
			saved.push({ id: 'p' + (saved.length + 1), kind: 'video', youtube_id: 'dQw4w9WgXcQ', child_id: null, title: '', pending: true });
			return route.fulfill({ json: {} });
		}
		return route.fulfill({ json: u.includes('/children') ? CHILDREN : u.includes('/policies') ? saved : [] });
	});
	await page.goto('/parent');
	const input = page.getByPlaceholder('Paste a YouTube link');
	await input.fill('https://youtu.be/dQw4w9WgXcQ');
	await page.getByRole('button', { name: 'Add', exact: true }).click();
	await expect(page.getByText('dQw4w9WgXcQ')).toBeVisible();
	await expect(page.getByText('pending')).toBeVisible();
	await expect(input).toHaveValue('');
	await page.reload();
	await expect(page.getByText('dQw4w9WgXcQ')).toBeVisible();
	await input.fill('https://youtu.be/second');
	await input.press('Enter');
	await expect.poll(() => posts.length).toBe(2);
});
