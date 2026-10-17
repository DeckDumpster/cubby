import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit()],
	server: {
		// In `npm run dev`, proxy API calls to the Axum server so the SPA and API share an origin.
		proxy: {
			'/api': 'http://localhost:8790',
			'/healthz': 'http://localhost:8790'
		}
	}
});
