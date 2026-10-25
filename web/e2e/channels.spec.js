import { test, expect } from '@playwright/test';

const tile = (id, title, ch = 'UC1', chTitle = 'Trains') => ({ youtube_id: id, title, channel_id: ch, channel_title: chTitle, thumb: `/thumbs/${id}.jpg` });
const CHANNELS = [
	{ channel_id: 'UC1', title: 'Trains', thumb: '/thumbs/v1.jpg' },
	{ channel_id: 'UC2', title: 'Boats', thumb: null }
];
const HOME = {
	child: { name: 'Kid', avatar: '🦄' },
	remaining: { videos: 3, minutes: null },
	channels: CHANNELS,
	sections: [{ key: 'all', title: 'All videos', tiles: [tile('v1', 'First video'), tile('v2', 'Second video')] }]
};

async function setup(page, { requested = false, canRequest = true } = {}) {
	const posts = [];
	let asked = requested;
	await page.addInitScript(() => localStorage.setItem('cubby_token', 'tok'));
	await page.route('**/api/child/**', async (route) => {
		const req = route.request();
		const path = new URL(req.url()).pathname;
		const json = (data) => route.fulfill({ json: data });
		if (path.endsWith('/home')) return json(HOME);
		if (path.endsWith('/channel/UC1/request')) {
			posts.push(path);
			asked = true;
			return json({ can_request: false, requested: true });
		}
		if (path.endsWith('/channel/UC1'))
			return json({
				channel: CHANNELS[0],
				tiles: [tile('v1', 'First video'), tile('v2', 'Second video')],
				can_request: canRequest && !asked,
				requested: asked
			});
		return route.fulfill({ status: 404, json: {} });
	});
	return posts;
}

test('the landing page has a Channels row linking to each channel page', async ({ page }, info) => {
	await setup(page);
	await page.goto('/watch');
	const row = page.getByTestId('channels-row');
	await expect(row).toContainText('Trains');
	await expect(row).toContainText('Boats');
	await page.screenshot({ path: info.outputPath('channels-row.png'), fullPage: true });
	await row.getByRole('link', { name: /Trains/ }).click();
	await expect(page).toHaveURL(/channel=UC1/);
	await expect(page.getByRole('heading', { name: 'Trains' })).toBeVisible();
	await expect(page.getByRole('button', { name: /First video/ })).toBeVisible();
});

test('the request button files one request and becomes the waiting state', async ({ page }, info) => {
	const posts = await setup(page);
	await page.goto('/watch?channel=UC1');
	const ask = page.getByTestId('channel-ask');
	await expect(ask).toHaveText('Ask for more from this channel');
	await page.screenshot({ path: info.outputPath('channel-page.png'), fullPage: true });
	await ask.click();
	await expect(page.getByTestId('channel-waiting')).toHaveText('Asked — waiting for a grown-up');
	await expect(ask).toHaveCount(0);
	await page.screenshot({ path: info.outputPath('channel-waiting.png'), fullPage: true });
	expect(posts).toHaveLength(1);
});

test('an imported channel shows no request button', async ({ page }) => {
	await setup(page, { canRequest: false });
	await page.goto('/watch?channel=UC1');
	await expect(page.getByRole('button', { name: /First video/ })).toBeVisible();
	await expect(page.getByTestId('channel-ask')).toHaveCount(0);
	await expect(page.getByTestId('channel-waiting')).toHaveCount(0);
});
