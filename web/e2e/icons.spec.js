import { test, expect } from '@playwright/test';

// Every icon URL answers with the right type and dimensions, the manifest parses and names them, and
// both /watch and /parent link them. The shot of the tab icon and the iPad icon goes to SHOTS_DIR.
const OUT = process.env.SHOTS_DIR ?? 'screenshots';

const pngSize = (b) => [b.readUInt32BE(16), b.readUInt32BE(20), b[25]]; // width, height, colour type

test('png icons have the right size and the apple icon has no alpha', async ({ request }) => {
	for (const [url, size, colour] of [
		['/assets/icons/apple-touch-icon-v1.png', 180, 2],
		['/assets/icons/icon-192-v1.png', 192, 6],
		['/assets/icons/icon-512-v1.png', 512, 6],
		['/assets/icons/icon-maskable-512-v1.png', 512, 2]
	]) {
		const res = await request.get(url);
		expect(res.headers()['content-type'], url).toBe('image/png');
		expect(pngSize(await res.body()), url).toEqual([size, size, colour]);
	}
});

test('favicon.ico holds 16, 32 and 48 and favicon.svg is an svg', async ({ request }) => {
	const ico = await request.get('/assets/icons/favicon-v1.ico');
	expect(ico.headers()['content-type']).toMatch(/icon/);
	const b = await ico.body();
	expect([b.readUInt16LE(2), b.readUInt16LE(4)]).toEqual([1, 3]);
	expect([0, 1, 2].map((i) => b[6 + i * 16])).toEqual([16, 32, 48]);
	const svg = await request.get('/assets/icons/favicon-v1.svg');
	expect(svg.headers()['content-type']).toMatch(/image\/svg\+xml/);
	expect(await svg.text()).toContain('<svg');
});

test('the manifest parses and every icon it names resolves', async ({ request }) => {
	const res = await request.get('/assets/manifest.webmanifest');
	expect(res.headers()['content-type']).toMatch(/manifest\+json|application\/json/);
	const m = await res.json();
	expect(m).toMatchObject({ name: 'Cubby', short_name: 'Cubby', display: 'standalone', start_url: '/watch' });
	expect(m.theme_color).toBe('#4f46e5');
	expect(m.icons.some((i) => i.purpose === 'maskable' && i.sizes === '512x512')).toBe(true);
	for (const i of m.icons) expect((await request.get(i.src)).status(), i.src).toBe(200);
});

for (const path of ['/watch', '/parent']) {
	test(`${path} links the icons and manifest`, async ({ page }) => {
		await page.route('**/api/**', (r) => r.fulfill({ status: 401, json: { error: 'x' } }));
		await page.goto(path);
		const href = (sel) => page.locator(sel).first().getAttribute('href');
		expect(await href('link[rel=icon][type="image/svg+xml"]')).toBe('/assets/icons/favicon-v1.svg');
		expect(await href('link[rel=apple-touch-icon]')).toBe('/assets/icons/apple-touch-icon-v1.png');
		expect(await href('link[rel=manifest]')).toBe('/assets/manifest.webmanifest');
	});
}

test('screenshot: browser tab and iPad home-screen icon', async ({ page }) => {
	await page.setContent(`<body style="margin:0;background:#dfe3ee;font:14px system-ui">
		<div style="display:flex;align-items:center;gap:8px;background:#fff;width:240px;padding:8px 12px;border-radius:10px 10px 0 0">
			<img src="/assets/icons/favicon-v1.svg" width="16" height="16"><span>Cubby</span></div>
		<div style="padding:24px;display:flex;gap:24px;align-items:flex-end">
			<div style="text-align:center"><img src="/assets/icons/apple-touch-icon-v1.png" width="180" height="180" style="border-radius:40px"><div>Cubby</div></div>
			<img src="/assets/icons/favicon-v1.ico" width="48" height="48"></div></body>`);
	await page.goto('/watch').catch(() => {});
	await page.screenshot({ path: `${OUT}/icons.png`, clip: { x: 0, y: 0, width: 360, height: 280 } });
});
