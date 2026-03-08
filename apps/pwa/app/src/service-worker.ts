/// <reference lib="WebWorker" />

import { build, files, version } from '$service-worker';

const sw = self as unknown as ServiceWorkerGlobalScope;
const CACHE = `krusty-pwa-${version}`;
const ASSETS = [...build, ...files];

sw.addEventListener('install', (event) => {
	event.waitUntil(
		(async () => {
			const cache = await caches.open(CACHE);
			await cache.addAll(ASSETS);
			await sw.skipWaiting();
		})()
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		(async () => {
			const keys = await caches.keys();
			await Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)));
			await sw.clients.claim();
		})()
	);
});

// Push notification handler
sw.addEventListener('push', (event) => {
	const data = event.data?.json() ?? { title: 'Krusty', body: 'Complete', session_id: null };
	event.waitUntil(
		sw.registration.showNotification(data.title, {
			body: data.body,
			icon: '/icon-192.png',
			badge: '/icon-192.png',
			tag: data.tag,
			data: { session_id: data.session_id, url: `/app?session=${data.session_id}` }
		})
	);
});

// Notification click — focus existing tab or open new window
sw.addEventListener('notificationclick', (event) => {
	event.notification.close();
	const sessionId = event.notification.data?.session_id;
	const targetUrl = event.notification.data?.url || '/app';

	event.waitUntil(
		sw.clients.matchAll({ type: 'window' }).then((clients) => {
			for (const client of clients) {
				if (client.url.includes('/app')) {
					client.postMessage({ type: 'notification-click', session_id: sessionId });
					return client.focus();
				}
			}
			return sw.clients.openWindow(targetUrl);
		})
	);
});

sw.addEventListener('fetch', (event) => {
	const { request } = event;
	if (request.method !== 'GET') {
		return;
	}

	const url = new URL(request.url);
	// Never cache API calls.
	if (url.pathname.startsWith('/api')) {
		return;
	}

	event.respondWith(
		(async () => {
			const cached = await caches.match(request);
			if (cached) {
				return cached;
			}

			try {
				const response = await fetch(request);
				if (response.ok && (url.origin === sw.location.origin || ASSETS.includes(url.pathname))) {
					const cache = await caches.open(CACHE);
					void cache.put(request, response.clone());
				}
				return response;
			} catch {
				return cached ?? Response.error();
			}
		})()
	);
});
