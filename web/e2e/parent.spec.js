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
				await page.getByRole('link', { name: new RegExp('^' + t) }).click();
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

const SECTIONS = [['allow', 'Allow-list', 'Allow a video'], ['review', 'Added by your policies', 'Added by your policies'], ['requests', 'Requests', 'Requests'], ['children', 'Children', 'Children'], ['history', 'History', 'History']];

test('each section is its own URL: open by URL or click, reload and back keep it', async ({ page }) => {
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		const body = u.includes('/history') ? HISTORY : u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES
			: u.includes('/review') ? REVIEW : u.includes('/requests') ? REQUESTS : {};
		return route.fulfill({ json: body });
	});
	const here = async (key, heading) => {
		await expect(page).toHaveURL(new RegExp(`/parent/${key}(\\?.*)?$`));
		await expect(page.getByRole('heading', { level: 2 })).toContainText(heading);
		await expect(page.locator('a.tab[aria-current="page"]')).toHaveAttribute('href', `/parent/${key}`);
	};
	await page.goto('/parent');
	await here('allow', 'Allow a video');
	for (const [key, label, heading] of SECTIONS) {
		await page.goto(`/parent/${key}`);
		await here(key, heading);
		await page.reload();
		await here(key, heading);
	}
	await page.goto('/parent/allow');
	await page.getByRole('link', { name: /^Children/ }).click();
	await here('children', 'Children');
	await page.getByRole('link', { name: /^History/ }).click();
	await here('history', 'History');
	await page.goBack();
	await here('children', 'Children');
	await page.goBack();
	await here('allow', 'Allow a video');
	await page.goForward();
	await here('children', 'Children');
});

test('the History child lives in the URL and survives a reload', async ({ page }) => {
	const asked = [];
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		if (u.includes('/history')) asked.push(u);
		return route.fulfill({ json: u.includes('/history') ? HISTORY : u.includes('/children') ? CHILDREN : [] });
	});
	await page.goto('/parent/history');
	await expect(page.getByRole('combobox')).toHaveValue('c1');
	await page.getByRole('combobox').selectOption('c2');
	await expect(page).toHaveURL(/\/parent\/history\?child=c2$/);
	await page.reload();
	await expect(page.getByRole('combobox')).toHaveValue('c2');
	expect(asked.at(-1)).toContain('/children/c2/history');
});

test('the child picture is picked from emoji when adding and editable from the card', async ({ page }) => {
	const kids = structuredClone(CHILDREN);
	const sent = [];
	await page.route('**/api/parent/**', async (route) => {
		const req = route.request();
		const u = req.url();
		if (req.method() !== 'GET' && u.includes('/children')) {
			sent.push({ method: req.method(), url: u, body: req.postDataJSON() });
			const b = req.postDataJSON();
			if (req.method() === 'POST') kids.push({ id: 'c3', devices: [], budget: CHILDREN[1].budget, ...b });
			else Object.assign(kids.find((k) => u.endsWith('/' + k.id)), b);
			return route.fulfill({ json: { ok: true, id: 'c3' } });
		}
		return route.fulfill({ json: u.includes('/children') ? kids : u.includes('/policies') ? POLICIES : [] });
	});
	await page.goto('/parent/children');
	await expect(page.getByRole('group', { name: 'Picture (shown on her screen)' })).toBeVisible();
	await page.getByPlaceholder('Name').fill('Lily');
	await page.getByRole('group', { name: 'Picture (shown on her screen)' }).getByRole('button', { name: '🦄' }).click();
	await page.getByRole('button', { name: 'Add', exact: true }).click();
	const card = page.locator('.card', { hasText: 'Lily' }).first();
	await expect(card.locator('.avatar')).toHaveText('🦄');
	expect(sent[0].body.avatar).toBe('🦄');
	await card.getByRole('button', { name: '🚀' }).click();
	await expect(card.locator('.avatar')).toHaveText('🚀');
	expect(sent[1].method).toBe('PUT');
});

test('History groups plays under a day heading with time-only rows linking to the video', async ({ page }) => {
	const plays = [
		{ id: 'g1', title: 'How volcanoes work', youtube_id: 'abc', granted_at: 1760131500, watched_s: 312, charge: 1, end_reason: 'ended', thumb: null },
		{ id: 'g2', title: 'Slime recipes', youtube_id: 'xyz', granted_at: 1760050000, watched_s: 40, charge: 0.5, end_reason: 'left', thumb: null },
		{ id: 'g3', title: 'Second that day', youtube_id: 'def', granted_at: 1760052000, watched_s: 9, charge: 0.1, end_reason: 'timeout', thumb: null }
	];
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		return route.fulfill({ json: u.includes('/history') ? plays : u.includes('/children') ? CHILDREN : [] });
	});
	for (const [w, h] of [[390, 844], [1280, 800]]) {
		await page.setViewportSize({ width: w, height: h });
		await page.goto('/parent/history');
		const days = page.getByTestId('day');
		await expect(days).toHaveCount(2);
		await expect(days.nth(0).getByRole('heading')).toHaveText('Fri Oct 10');
		await expect(days.nth(1).getByRole('heading')).toHaveText('Thu Oct 9');
		await expect(days.nth(1).getByTestId('play')).toHaveCount(2);
		const row = days.nth(0).getByTestId('play');
		await expect(row.locator('time')).toHaveText(/^\d{1,2}:\d{2} [AP]M$/);
		await expect(row).not.toContainText('2025');
		const link = row.getByRole('link', { name: 'How volcanoes work' });
		await expect(link).toHaveAttribute('href', 'https://www.youtube.com/watch?v=abc');
		await expect(link).toHaveAttribute('target', '_blank');
		await expect(link).toHaveAttribute('rel', 'noopener');
		await expect(row).toContainText('Finished the video');
		expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
		if (shots) await page.screenshot({ path: `${shots}/history-days-${w}.png`, fullPage: true });
	}
});

for (const scheme of ['light', 'dark'])
	for (const [w, h] of [[390, 844], [1280, 800]])
		for (const n of [0, 1, 3])
			test(`children: Pair a device clears the device list (${n} devices, ${scheme} ${w}px)`, async ({ page }) => {
				await page.emulateMedia({ colorScheme: scheme });
				await page.setViewportSize({ width: w, height: h });
				const kids = [{ ...CHILDREN[1], devices: Array.from({ length: n }, (_, i) => ({ id: 'd' + i, label: 'Device ' + i, revoked_at: null })) }];
				await page.route('**/api/parent/**', (route) => route.fulfill({ json: route.request().url().includes('/children') ? kids : [] }));
				await page.goto('/parent/children');
				const btn = page.getByRole('button', { name: 'Pair a device' });
				await expect(btn).toBeVisible();
				const gap = await page.evaluate(() => {
					const ul = document.querySelector('.card ul.list');
					const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Pair a device');
					const token = parseFloat(getComputedStyle(document.documentElement).fontSize) * 0.75;
					return { gap: b.getBoundingClientRect().top - ul.getBoundingClientRect().bottom, token };
				});
				expect(gap.gap).toBeGreaterThanOrEqual(gap.token - 0.5);
				if (shots) await page.screenshot({ path: `${shots}/children-${n}dev-${scheme}-${w}.png`, fullPage: true });
			});

const PROGRESS = [
	{ child_id: 'c1', tz: 'America/Chicago', videos: { limit: 3, used: 1.5, left: 1.5, reached: false }, minutes: { limit: 60, used: 15, left: 45, reached: false },
		resets_at: 1792900000, window: { restricted: false, open: true, opens_at: null, closes_at: null },
		watching: { youtube_id: 'abc', title: 'How volcanoes work', since: 1792880000, minutes: 2 } },
	{ child_id: 'c2', tz: 'America/Chicago', videos: { limit: 3, used: 3, left: 0, reached: true }, minutes: { limit: null, used: 12, left: null, reached: false },
		resets_at: 1792900000, window: { restricted: true, open: false, opens_at: 1792910000, closes_at: null }, watching: null }
];

test('the progress card shows used and left against the limits and who is watching', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 844 });
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		const body = u.includes('/progress') ? PROGRESS : u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES : [];
		return route.fulfill({ json: body });
	});
	await page.goto('/parent');
	const maya = page.getByRole('article', { name: 'Maya progress' });
	await expect(maya).toContainText('Watching');
	await expect(maya).not.toContainText('How volcanoes work');
	await maya.getByRole('button', { name: /Watching/ }).click();
	await expect(maya).toContainText('How volcanoes work');
	await expect(maya).toContainText('1.50 left');
	await expect(maya).toContainText('45 left');
	await expect(maya).toContainText('15 of 60 used');
	const theo = page.getByRole('article', { name: 'Theo progress' });
	await expect(theo).toContainText('limit reached');
	await expect(theo).toContainText('No limit');
	await expect(theo).toContainText('Viewing is closed');
	await expect(page.locator('body')).not.toContainText('Extra lasts until midnight');
	await expect(page.locator('body')).not.toContainText('Resets at midnight');
	const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
	expect(overflow).toBe(false);
	if (shots) await page.screenshot({ path: `${shots}/progress-390.png`, fullPage: true });
});

for (const scheme of ['light', 'dark'])
	test(`progress bars fill to the share used (${scheme} 390px)`, async ({ page }) => {
		await page.emulateMedia({ colorScheme: scheme });
		await page.setViewportSize({ width: 390, height: 844 });
		const rows = [
			{ ...PROGRESS[0], child_id: 'c1', videos: { limit: 4, used: 1, left: 3, reached: false }, minutes: { limit: 25, used: 14.5, left: 10.5, reached: false } },
			{ ...PROGRESS[0], child_id: 'c2', watching: null, videos: { limit: 4, used: 0, left: 4, reached: false }, minutes: { limit: 25, used: 25, left: 0, reached: true } }
		];
		await page.route('**/api/parent/**', (route) => {
			const u = route.request().url();
			return route.fulfill({ json: u.includes('/progress') ? rows : u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES : [] });
		});
		await page.goto('/parent/children');
		const fill = async (who, label) => {
			const bar = page.getByRole('article', { name: `${who} progress` }).getByRole('progressbar', { name: `${label} used` });
			await expect(bar).toBeVisible();
			return bar.evaluate((el) => {
				const span = el.firstElementChild;
				const sr = span.getBoundingClientRect(), tr = el.getBoundingClientRect();
				return { ratio: sr.width / tr.width, covers: sr.height >= tr.height - 1 && sr.height >= 8 && sr.top >= tr.top - 1 && sr.bottom <= tr.bottom + 1, tall: tr.height, bg: getComputedStyle(span).backgroundColor, track: getComputedStyle(el).backgroundColor };
			});
		};
		const partial = await fill('Maya', 'Minutes');
		expect(Math.abs(partial.ratio - 14.5 / 25)).toBeLessThan(0.03);
		expect(partial.covers, `fill must span the track (track ${partial.tall}px tall)`).toBe(true);
		expect(partial.bg).not.toBe(partial.track);
		expect(partial.bg).not.toBe('rgba(0, 0, 0, 0)');
		expect((await fill('Theo', 'Videos')).ratio).toBe(0);
		const full = await fill('Theo', 'Minutes');
		expect(full.ratio).toBeGreaterThan(0.99);
		expect(full.bg).not.toBe(partial.bg);
		if (shots) await page.screenshot({ path: `${shots}/progress-bars-${scheme}-390.png`, fullPage: true });
	});

test('watching collapses into the title line; cards match height and the expansion survives a poll', async ({ page }) => {
	await page.setViewportSize({ width: 390, height: 844 });
	await page.route('**/api/parent/**', (route) => {
		const u = route.request().url();
		return route.fulfill({ json: u.includes('/progress') ? PROGRESS : u.includes('/children') ? CHILDREN : u.includes('/policies') ? POLICIES : [] });
	});
	await page.clock.install();
	await page.goto('/parent');
	const maya = page.getByRole('article', { name: 'Maya progress' });
	const theo = page.getByRole('article', { name: 'Theo progress' });
	const head = (a) => a.locator('h3').evaluate((h) => h.getBoundingClientRect().height);
	expect(await head(maya)).toBe(await head(theo));
	const toggle = maya.getByRole('button', { name: /Watching/ });
	await expect(toggle).toHaveAttribute('aria-expanded', 'false');
	await expect(maya).not.toContainText('min in');
	if (shots) await page.screenshot({ path: `${shots}/progress-collapsed-390.png`, fullPage: true });
	await toggle.click();
	await expect(toggle).toHaveAttribute('aria-expanded', 'true');
	await expect(maya).toContainText('How volcanoes work');
	await expect(maya).toContainText('2 min in');
	await page.clock.runFor(16000);
	await expect(toggle).toHaveAttribute('aria-expanded', 'true');
	await expect(maya).toContainText('How volcanoes work');
	expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
	if (shots) await page.screenshot({ path: `${shots}/progress-expanded-390.png`, fullPage: true });
	await toggle.click();
	await expect(maya).not.toContainText('min in');
});

test('a channel shows a live allowed/excluded count before its filters are saved', async ({ page }) => {
	const policy = { ...POLICIES[0], rules: { exclude_shorts: false, min_minutes: null, max_minutes: null, title_has: [], title_lacks: [] }, allowed_count: 3, excluded_count: 0 };
	const puts = [];
	await page.route('**/api/parent/**', (route) => {
		const r = route.request();
		const u = r.url();
		if (u.endsWith('/rules/preview'))
			return route.fulfill({ json: { allowed: 1, excluded: 2, excluded_videos: [{ youtube_id: 's', title: 'Quick clip', duration_s: 45 }, { youtube_id: 'c', title: 'Big compilation', duration_s: 5400 }] } });
		if (r.method() === 'PUT' && u.endsWith('/rules')) {
			puts.push(r.postDataJSON());
			return route.fulfill({ json: {} });
		}
		return route.fulfill({ json: u.includes('/children') ? CHILDREN : u.includes('/policies') ? [policy] : u.includes('/library') ? { groups: [] } : [] });
	});
	await page.goto('/parent/allow');
	await page.getByText('Filters', { exact: false }).first().click();
	await page.getByLabel('Exclude Shorts').check();
	await page.getByLabel('Longest, minutes').fill('30');
	await expect(page.getByText('not saved yet')).toContainText('1 allowed · 2 excluded');
	await page.getByText('Excluded videos').click();
	await expect(page.getByText('Big compilation')).toBeVisible();
	await page.getByRole('button', { name: 'Save filters' }).click();
	await expect.poll(() => puts.length).toBe(1);
	expect(puts[0]).toMatchObject({ exclude_shorts: true, max_minutes: 30, min_minutes: null });
});
