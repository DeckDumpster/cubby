// The user path, end to end, in a real browser: a parent sets Cubby up, a child's device pairs and
// watches. It runs two ways from one file:
//   - e2e:full  against a stack the harness starts (real server binary, built web app, Pocket ID
//     stubbed by a local OIDC provider, YouTube by a local fake), every step;
//   - e2e:smoke against a deployed instance (SMOKE_BASE_URL), where what a step needs and the
//     environment lacks is skipped with its reason printed.
import { spawnSync } from 'node:child_process';
import { test, expect } from '@playwright/test';
import { PARENT_EMAIL, VIDEOS } from './stack.js';

const smoke = !!process.env.SMOKE_BASE_URL;
const VOLCANO = 'VOLCANO0001';
const SLIME = 'SLIME000002';

/** A stand-in for the YouTube IFrame API: the browser has no route to youtube.com here. */
const FAKE_PLAYER = () => {
	window.YT = {
		PlayerState: { ENDED: 0, PLAYING: 1, PAUSED: 2 },
		Player: class {
			constructor(el) {
				el.innerHTML = '<iframe title="fake player"></iframe>';
			}
			getCurrentTime() {
				return 12;
			}
			destroy() {}
		}
	};
};

/** The page carries a stylesheet with rules in it: the unstyled /parent shipped once. */
async function expectStyled(page) {
	const rules = await page.evaluate(() =>
		[...document.styleSheets].reduce((n, s) => {
			try {
				return n + s.cssRules.length;
			} catch {
				return n;
			}
		}, 0)
	);
	expect(rules, 'the page has no stylesheet rules applied').toBeGreaterThan(0);
}

function skipped(reason) {
	console.log(`SKIPPED: ${reason}`);
	test.info().annotations.push({ type: 'skipped', description: reason });
}

test.describe('hermetic stack', () => {
	test.skip(smoke, 'the full walk needs the fake YouTube and Pocket ID of the harness');

	test('a parent sets up; a child pairs, sees the allowed video and plays it', async ({ browser, baseURL }) => {
		const parentCtx = await browser.newContext({ baseURL });
		const parent = await parentCtx.newPage();
		let code = '';

		await test.step('the parent signs in through the provider and loads /parent, styled, as a parent of a family', async () => {
			const children = parent.waitForResponse((r) => r.url().endsWith('/api/parent/children') && r.request().method() === 'GET');
			await parent.goto('/parent');
			await parent.waitForURL(/\/parent\/allow/);
			expect((await children).status(), `${PARENT_EMAIL} is not accepted as a parent`).toBe(200);
			await expect(parent.getByRole('heading', { name: /Cubby/ })).toBeVisible();
			await expectStyled(parent);
			await expect(parent.getByRole('alert')).toHaveCount(0);
		});

		await test.step('adds a child and gets a pairing code', async () => {
			await parent.getByRole('link', { name: /^Children/ }).click();
			await parent.getByPlaceholder('Name').fill('Maya');
			await parent.getByRole('button', { name: 'Add', exact: true }).click();
			await expect(parent.getByText('Maya').first()).toBeVisible();
			await parent.getByRole('button', { name: 'Pair a device' }).click();
			const shown = parent.getByLabel('Pairing code');
			await expect(shown).toHaveText(/\S+/);
			code = (await shown.textContent()).trim();
		});

		await test.step('allows one video', async () => {
			await parent.getByRole('link', { name: /^Allow-list/ }).click();
			await parent.getByPlaceholder('Paste a YouTube link').fill(`https://www.youtube.com/watch?v=${VOLCANO}`);
			await parent.getByRole('button', { name: 'Add', exact: true }).click();
			await expect(parent.locator('ul.list li').filter({ hasText: VIDEOS[VOLCANO].title })).toBeVisible();
			await expect(parent.getByRole('alert')).toHaveCount(0);
		});

		const childCtx = await browser.newContext({ baseURL });
		await childCtx.addInitScript(FAKE_PLAYER);
		const child = await childCtx.newPage();

		await test.step('the device pairs on /watch', async () => {
			await child.goto('/watch');
			await expectStyled(child);
			await child.getByLabel('pairing code').fill(code);
			await child.getByRole('button', { name: 'Go' }).click();
			await expect(child.getByLabel('pairing code')).toHaveCount(0);
		});

		await test.step('home shows the allowed video, and only it', async () => {
			await expect(child.getByRole('button', { name: VIDEOS[VOLCANO].title }).first()).toBeVisible();
			await expect(child.getByText(VIDEOS[SLIME].title)).toHaveCount(0);
			await expect
				.poll(() => child.locator('img.thumb').evaluateAll((imgs) => imgs.length > 0 && imgs.every((i) => i.naturalWidth > 0)))
				.toBe(true);
		});

		await test.step('search finds the allowed video and not the other', async () => {
			await child.getByLabel('search').fill('volcano');
			await child.getByRole('button', { name: 'Search' }).click();
			await expect(child.getByRole('heading', { name: 'Videos about volcano' })).toBeVisible();
			await expect(child.getByRole('button', { name: VIDEOS[VOLCANO].title }).first()).toBeVisible();
			await child.getByLabel('search').fill('slime');
			await child.getByRole('button', { name: 'Search' }).click();
			await expect(child.getByText('Nothing found.')).toBeVisible();
			await child.getByLabel('search').fill('volcano');
			await child.getByRole('button', { name: 'Search' }).click();
		});

		await test.step('playback starts: a play grant is created', async () => {
			const play = child.waitForResponse((r) => r.url().endsWith('/api/child/play'));
			await child.getByRole('button', { name: VIDEOS[VOLCANO].title }).first().click();
			const res = await play;
			expect(res.status()).toBe(200);
			expect((await res.json()).grant_id, 'no play grant was created').toBeTruthy();
			await expect(child.getByRole('heading', { name: VIDEOS[VOLCANO].title })).toBeVisible();
			await expect(child.locator('iframe[title="fake player"]')).toBeAttached();
		});

		await test.step('the grown-up control unpairs the device and returns to pairing', async () => {
			await child.goto('/watch');
			child.once('dialog', (d) => d.accept());
			await child.getByRole('button', { name: /unpair this device/ }).click();
			await expect(child.getByLabel('pairing code')).toBeVisible();
			expect(await child.evaluate(() => localStorage.getItem('cubby_token'))).toBeNull();
		});

		await test.step('revoking a paired device sends its /watch back to pairing', async () => {
			await parent.getByRole('link', { name: /^Children/ }).click();
			await parent.getByRole('button', { name: 'Pair a device' }).click();
			const shown = parent.getByLabel('Pairing code');
			await expect(shown).toHaveText(/\S+/);
			await expect(shown).not.toHaveText(code);
			const fresh = (await shown.textContent()).trim();
			await child.goto('/watch');
			await child.getByLabel('pairing code').fill(fresh);
			await child.getByRole('button', { name: 'Go' }).click();
			await expect(child.getByLabel('search')).toBeVisible();
			await parent.reload();
			const revoke = parent.getByRole('button', { name: 'Revoke' });
			await expect(revoke.first()).toBeVisible();
			for (let n = await revoke.count(); n > 0; n--) await revoke.first().click();
			await expect(revoke).toHaveCount(0);
			await child.goto('/watch');
			await expect(child.getByLabel('pairing code')).toBeVisible();
		});

		await test.step('Sign out ends the parent session', async () => {
			await expect(parent.getByRole('link', { name: 'Sign out' })).toHaveAttribute('href', '/auth/logout');
			expect((await parentCtx.request.get('/api/parent/children')).status()).toBe(200);
			await parent.getByRole('link', { name: 'Sign out' }).click();
			expect((await parentCtx.request.get('/api/parent/children')).status()).toBe(401);
		});
	});
});

test.describe('smoke walk, pointed at the harness stack', () => {
	test.skip(smoke, 'the smoke walk is run by deploy/smoke.sh in this mode');

	// The deploy's own check must itself work: run it here with a paired device.
	test('passes, and walks both the parent and the child steps', async ({ baseURL, request }) => {
		// The request context follows /auth/login through the fake provider and keeps the session cookie.
		await request.get('/auth/login');
		const kid = await (await request.post('/api/parent/children', { data: { name: 'Smoke', avatar: '', tz: 'UTC' } })).json();
		const kids = await (await request.get('/api/parent/children')).json();
		const id = kid.id ?? kids.find((c) => c.name === 'Smoke').id;
		const { code } = await (await request.post(`/api/parent/children/${id}/pairing`)).json();
		const { token } = await (await request.post('/api/child/pair', { data: { code } })).json();
		const r = spawnSync('npx', ['playwright', 'test', '-c', 'playwright.smoke.config.js'], {
			encoding: 'utf8',
			env: { ...process.env, SMOKE_BASE_URL: baseURL, SMOKE_DEVICE_TOKEN: token }
		});
		console.log(r.stdout, r.stderr);
		expect(r.status, 'the smoke walk failed against the harness stack').toBe(0);
		expect(r.stdout).toContain('parent steps: /parent sends a signed-out browser to the provider');
		expect(r.stdout).toContain('child steps: paired device loaded home');
	});
});

test.describe('deployed instance', () => {
	test.skip(!smoke, 'the smoke walk needs SMOKE_BASE_URL');

	// A deployed instance has no test identity to sign in as, so the parent steps check what a
	// signed-out browser meets: the parent API refuses, /parent redirects to sign-in, and sign-in
	// reaches the real provider's authorize endpoint (which needs discovery to work).
	test('parent steps', async ({ baseURL, request }) => {
		expect((await request.get('/api/parent/children')).status(), '/api/parent answers a signed-out caller').toBe(401);
		const page = await request.get('/parent/allow', { maxRedirects: 0 });
		expect(page.status()).toBe(303);
		expect(page.headers().location).toMatch(/^\/auth\/login\?next=/);
		const login = await request.get('/auth/login', { maxRedirects: 0 });
		expect(login.status(), 'sign-in could not reach the provider (discovery failed?)').toBe(303);
		const to = new URL(login.headers().location);
		expect(to.pathname).toMatch(/authorize/);
		expect(to.searchParams.get('redirect_uri')).toBe(`${baseURL}/auth/callback`);
		expect(to.searchParams.get('scope')).toBe('openid email profile groups');
		console.log('parent steps: /parent sends a signed-out browser to the provider');
		skipped('signing in and the writes behind it are not run: a deploy has no parent to sign in as');
	});

	test('child steps', async ({ browser, baseURL, request }) => {
		const page = await (await browser.newContext({ baseURL })).newPage();
		await page.goto('/watch');
		await expectStyled(page);
		// An unmounted route answers 404 (or the SPA shell with 200); a mounted one asks for a token.
		for (const path of ['/api/child/home', '/api/child/search?q=x', '/api/child/next?after=x']) {
			const r = await request.get(path);
			expect(r.status(), `${path} is not mounted behind the device guard`).toBe(401);
		}
		const tok = process.env.SMOKE_DEVICE_TOKEN;
		if (!tok) return skipped('SMOKE_DEVICE_TOKEN unset: no paired device, so home, search and playback are not walked');
		await page.addInitScript((t) => localStorage.setItem('cubby_token', t), tok);
		const home = page.waitForResponse((r) => r.url().endsWith('/api/child/home'));
		await page.goto('/watch');
		expect((await home).status()).toBe(200);
		await expect(page.getByLabel('search')).toBeVisible();
		console.log('child steps: paired device loaded home');
		skipped('starting playback is not run: it would charge the real child a play');
	});
});
