import { test, expect } from '@playwright/test';
import { createServer } from 'node:http';

// A stand-in API on the dev server's proxy target that answers the way the old SPA fallback did:
// cacheable, with Last-Modified. A device that cached such a response must still ask again.
test.describe.configure({ mode: 'serial' });

test('a primed HTTP cache for /api/child/home does not survive a reload', async ({ page }) => {
	let hits = 0;
	const server = createServer((req, res) => {
		if (req.url === '/api/child/home') hits++;
		res.writeHead(200, {
			'content-type': 'application/json',
			'last-modified': new Date(Date.now() - 86400e3).toUTCString(),
			'cache-control': 'max-age=3600'
		});
		res.end(
			JSON.stringify({ child: { name: 'Kid' }, remaining: { videos: 1, minutes: null }, total_videos: 0, sections: [] })
		);
	});
	await new Promise((r) => server.listen(8790, '127.0.0.1', r));
	try {
		await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
		await page.goto('/');
		await expect.poll(() => hits).toBeGreaterThan(0);
		// the stale entry an old device holds: stored by a plain fetch, which the server allows
		await page.evaluate(() => fetch('/api/child/home').then((r) => r.text()));
		await page.evaluate(() => fetch('/api/child/home').then((r) => r.text()));
		const primed = hits;
		await page.reload();
		await expect.poll(() => hits).toBeGreaterThan(primed);
	} finally {
		await new Promise((r) => server.close(r));
	}
});
