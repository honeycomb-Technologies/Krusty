import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit()],
	build: {
		target: 'esnext',
		// Optional Shiki language-pack chunks are emitted as separate dynamic assets.
		// Keep the warning signal focused on app-shell regressions rather than those vendor payloads.
		chunkSizeWarningLimit: 800
	},
	optimizeDeps: {
		esbuildOptions: {
			target: 'esnext'
		}
	},
	server: {
		host: true,
		port: 5173,
		proxy: {
			'/api': {
				target: process.env.KRUSTY_SERVER_ORIGIN || 'http://localhost:3000',
				changeOrigin: true,
				secure: false
			},
			'/ws': {
				target: process.env.KRUSTY_SERVER_ORIGIN || 'http://localhost:3000',
				ws: true,
				secure: false
			}
		}
	}
});
