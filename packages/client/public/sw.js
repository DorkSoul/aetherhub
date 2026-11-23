// Makes Aetherhub installable as an app. It caches nothing: the game needs the server, and a stale
// cached client would be out of step with it after a redeploy.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));
