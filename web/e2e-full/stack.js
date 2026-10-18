// The stack the end-to-end walk runs against: the real cubby-server binary serving the built web
// app, with Pocket ID stood in for by a local OIDC provider and YouTube by a local fake Data API.
import { spawn } from 'node:child_process';
import { generateKeyPairSync, createSign } from 'node:crypto';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { mkdtempSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

export const PARENT_EMAIL = 'parent@example.test';
export const VIDEOS = {
	VOLCANO0001: { title: 'Volcano experiments at home', channel: 'UCscience0001', channelTitle: 'Science for Kids' },
	SLIME000002: { title: 'Slime recipes', channel: 'UCcrafty0002', channelTitle: 'Crafty' },
	PUPS0000003: { title: 'Pups save the day', channel: 'UCpups000003', channelTitle: "Skye's Pup Squad" }
};
// The channel a parent adds by @handle: the Data API finds it only through forHandle, and a
// channels?id= lookup of the handle itself returns nothing (the bug this walk guards).
export const HANDLE = { handle: '@SkyesPupSquad', id: 'UCpups000003', uploads: 'UUpups000003', video: 'PUPS0000003' };
// 1x1 JPEG
const JPEG = Buffer.from(
	'/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA=',
	'base64'
);

const b64u = (b) => Buffer.from(b).toString('base64url');

function listen(server, port) {
	return new Promise((res, rej) => {
		server.once('error', rej);
		server.listen(port, '127.0.0.1', () => res(server.address().port));
	});
}

export const CLIENT_ID = 'e2e-client';
export const CLIENT_SECRET = 'e2e-secret';

/**
 * A stand-in for Pocket ID: discovery, an /authorize that approves at once as `email` in the
 * required group (a browser sees only redirects), a /token that checks the client secret and PKCE,
 * and the JWKS for the RS256 ID tokens it signs.
 */
function fakeOidc(email, kid = 'e2e-key') {
	const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
	const jwk = { ...publicKey.export({ format: 'jwk' }), kid, alg: 'RS256', use: 'sig' };
	const codes = new Map();
	let n = 0;
	// What the provider itself remembers: /authorize signs the browser in, end-session signs it out.
	const seen = { authorizes: 0, signedIn: false };
	const server = createServer(async (req, res) => {
		const u = new URL(req.url, 'http://x');
		const issuer = `http://127.0.0.1:${server.address().port}`;
		const json = (o, status = 200) => (res.writeHead(status, { 'content-type': 'application/json' }), res.end(JSON.stringify(o)));
		if (u.pathname === '/.well-known/openid-configuration') {
			return json({ issuer, authorization_endpoint: `${issuer}/authorize`, token_endpoint: `${issuer}/token`, jwks_uri: `${issuer}/jwks`, end_session_endpoint: `${issuer}/logout` });
		}
		if (u.pathname === '/jwks') return json({ keys: [jwk] });
		if (u.pathname === '/__state') return json(seen);
		if (u.pathname === '/logout') {
			const q = u.searchParams;
			if (!q.get('id_token_hint') || !q.get('post_logout_redirect_uri')) return json({ error: 'invalid_request' }, 400);
			seen.signedIn = false;
			res.writeHead(302, { location: q.get('post_logout_redirect_uri') });
			return res.end();
		}
		if (u.pathname === '/authorize') {
			const q = u.searchParams;
			seen.authorizes++;
			seen.signedIn = true;
			const code = `code-${++n}`;
			codes.set(code, { nonce: q.get('nonce'), challenge: q.get('code_challenge') });
			const back = new URL(q.get('redirect_uri'));
			back.searchParams.set('code', code);
			back.searchParams.set('state', q.get('state'));
			res.writeHead(302, { location: back.toString() });
			return res.end();
		}
		if (u.pathname === '/token' && req.method === 'POST') {
			let body = '';
			for await (const c of req) body += c;
			const f = new URLSearchParams(body);
			const issued = codes.get(f.get('code'));
			codes.delete(f.get('code'));
			const challenge = createHash('sha256').update(f.get('code_verifier') ?? '').digest('base64url');
			if (f.get('client_id') !== CLIENT_ID || f.get('client_secret') !== CLIENT_SECRET || !issued || issued.challenge !== challenge) {
				return json({ error: 'invalid_grant' }, 400);
			}
			const now = Math.floor(Date.now() / 1000);
			const head = b64u(JSON.stringify({ alg: 'RS256', typ: 'JWT', kid }));
			const claims = b64u(JSON.stringify({ iss: issuer, aud: CLIENT_ID, sub: 'e2e', email, groups: ['cubby-parents'], nonce: issued.nonce, iat: now, exp: now + 300 }));
			const sig = createSign('RSA-SHA256').update(`${head}.${claims}`).sign(privateKey);
			return json({ id_token: `${head}.${claims}.${b64u(sig)}`, access_token: 'x', token_type: 'Bearer' });
		}
		res.writeHead(404).end();
	});
	return server;
}

function fakeYouTube() {
	const calls = [];
	const server = createServer((req, res) => {
		const u = new URL(req.url, 'http://x');
		calls.push(u.pathname + u.search);
		const json = (o) => (res.setHeader('content-type', 'application/json'), res.end(JSON.stringify(o)));
		if (u.pathname.startsWith('/thumb/')) return (res.setHeader('content-type', 'image/jpeg'), res.end(JPEG));
		if (u.pathname.endsWith('/videos')) {
			const base = `http://127.0.0.1:${server.address().port}`;
			const items = (u.searchParams.get('id') ?? '')
				.split(',')
				.filter((id) => VIDEOS[id])
				.map((id) => ({
					id,
					snippet: {
						title: VIDEOS[id].title,
						channelId: VIDEOS[id].channel,
						channelTitle: VIDEOS[id].channelTitle,
						tags: ['e2e'],
						publishedAt: '2026-01-01T00:00:00Z',
						thumbnails: { medium: { url: `${base}/thumb/${id}.jpg` } }
					},
					contentDetails: { duration: 'PT4M13S' },
					status: { privacyStatus: 'public', madeForKids: true }
				}));
			return json({ items });
		}
		if (u.pathname.endsWith('/channels')) {
			if (u.searchParams.get('forHandle') === HANDLE.handle)
				return json({ items: [{ id: HANDLE.id, snippet: { title: VIDEOS[HANDLE.video].channelTitle } }] });
			if (u.searchParams.get('id') === HANDLE.id)
				return json({ items: [{ id: HANDLE.id, contentDetails: { relatedPlaylists: { uploads: HANDLE.uploads } } }] });
			return json({ items: [] });
		}
		if (u.pathname.endsWith('/playlistItems') && u.searchParams.get('playlistId') === HANDLE.uploads)
			return json({ items: [{ contentDetails: { videoId: HANDLE.video } }] });
		json({ items: [] });
	});
	return { server, calls };
}

/** Starts the fakes and the server binary; resolves to the base URL and a stop function. */
export async function startStack({ ports = {} } = {}) {
	const web = resolve(import.meta.dirname, '../build');
	const bin = process.env.CUBBY_SERVER_BIN ?? join(process.env.CARGO_TARGET_DIR ?? resolve(import.meta.dirname, '../../target'), 'debug/cubby-server');
	if (!existsSync(join(web, 'index.html'))) throw new Error(`no built web app at ${web}; run npm run build`);
	if (!existsSync(bin)) throw new Error(`no server binary at ${bin}; run cargo build -p cubby-server (or set CUBBY_SERVER_BIN)`);

	const oidc = fakeOidc(PARENT_EMAIL);
	const yt = fakeYouTube();
	const oidcPort = await listen(oidc, ports.oidc ?? 0);
	const ytPort = await listen(yt.server, ports.yt ?? 0);

	const port = ports.server ?? 4180;
	const dataDir = mkdtempSync(join(tmpdir(), 'cubby-e2e-'));
	const child = spawn(bin, [], {
		env: {
			PATH: process.env.PATH,
			CUBBY_BIND: '127.0.0.1',
			CUBBY_PORT: String(port),
			CUBBY_WEB_DIR: web,
			CUBBY_DATA_DIR: dataDir,
			CUBBY_OIDC_ISSUER: `http://127.0.0.1:${oidcPort}`,
			CUBBY_OIDC_CLIENT_ID: CLIENT_ID,
			CUBBY_OIDC_CLIENT_SECRET: CLIENT_SECRET,
			CUBBY_OIDC_BASE_URL: `http://127.0.0.1:${port}`,
			CUBBY_BOOTSTRAP_PARENT: PARENT_EMAIL,
			CUBBY_YT_API_KEY: 'fake-key',
			CUBBY_YT_BASE_URL: `http://127.0.0.1:${ytPort}/youtube/v3`
		},
		stdio: ['ignore', 'inherit', 'inherit']
	});
	let exited = false;
	child.on('exit', () => (exited = true));
	const baseURL = `http://127.0.0.1:${port}`;
	for (let i = 0; i < 100; i++) {
		if (exited) throw new Error('cubby-server exited during start');
		try {
			if ((await fetch(`${baseURL}/healthz`)).ok) break;
		} catch {
			/* not listening yet */
		}
		await new Promise((r) => setTimeout(r, 100));
		if (i === 99) throw new Error('cubby-server never answered /healthz');
	}
	const stop = async () => {
		child.kill('SIGINT');
		oidc.close();
		yt.server.close();
	};
	return { baseURL, ytCalls: yt.calls, stop };
}
