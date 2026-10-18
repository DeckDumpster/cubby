// The stack the end-to-end walk runs against: the real cubby-server binary serving the built web
// app, with Cloudflare Access stood in for by a local JWKS and YouTube by a local fake Data API.
import { spawn } from 'node:child_process';
import { generateKeyPairSync, createSign } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdtempSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

export const PARENT_EMAIL = 'parent@example.test';
export const VIDEOS = {
	VOLCANO0001: { title: 'Volcano experiments at home', channel: 'UCscience0001', channelTitle: 'Science for Kids' },
	SLIME000002: { title: 'Slime recipes', channel: 'UCcrafty0002', channelTitle: 'Crafty' }
};
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

/** A signer and the JWKS document that verifies it. */
export function accessKeys(kid = 'e2e-key') {
	const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
	const jwk = { ...publicKey.export({ format: 'jwk' }), kid, alg: 'RS256', use: 'sig' };
	const sign = ({ iss, aud, email }) => {
		const now = Math.floor(Date.now() / 1000);
		const head = b64u(JSON.stringify({ alg: 'RS256', typ: 'JWT', kid }));
		const body = b64u(JSON.stringify({ iss, aud, email, iat: now, nbf: now - 60, exp: now + 3600 }));
		const sig = createSign('RSA-SHA256').update(`${head}.${body}`).sign(privateKey);
		return `${head}.${body}.${b64u(sig)}`;
	};
	return { jwks: { keys: [jwk] }, sign };
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

	const keys = accessKeys();
	const jwks = createServer((_, res) => (res.setHeader('content-type', 'application/json'), res.end(JSON.stringify(keys.jwks))));
	const yt = fakeYouTube();
	const jwksPort = await listen(jwks, ports.jwks ?? 0);
	const ytPort = await listen(yt.server, ports.yt ?? 0);

	const team = 'https://e2e.cloudflareaccess.test';
	const aud = 'e2e-aud';
	const port = ports.server ?? 4180;
	const dataDir = mkdtempSync(join(tmpdir(), 'cubby-e2e-'));
	const child = spawn(bin, [], {
		env: {
			PATH: process.env.PATH,
			CUBBY_BIND: '127.0.0.1',
			CUBBY_PORT: String(port),
			CUBBY_WEB_DIR: web,
			CUBBY_DATA_DIR: dataDir,
			CUBBY_ACCESS_TEAM_DOMAIN: team,
			CUBBY_ACCESS_AUD: aud,
			CUBBY_ACCESS_JWKS_URL: `http://127.0.0.1:${jwksPort}/`,
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
		jwks.close();
		yt.server.close();
	};
	return { baseURL, parentJwt: () => keys.sign({ iss: team, aud, email: PARENT_EMAIL }), ytCalls: yt.calls, stop };
}
