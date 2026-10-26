import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [
		sveltekit(),
		{
			// The server serves /native/player from static/native/player.html; do the same in dev.
			name: 'native-player-route',
			enforce: 'pre',
			configureServer(server) {
				server.middlewares.use((req, _res, next) => {
					if (req.url === '/native/player') req.url = '/native/player.html';
					next();
				});
			}
		}
	],
	server: {
		// In `npm run dev`, proxy API calls to the Axum server so the SPA and API share an origin.
		proxy: {
			'/api': 'http://localhost:8790',
			'/healthz': 'http://localhost:8790'
		}
	}
});
