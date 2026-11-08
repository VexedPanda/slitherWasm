// Service worker that makes the site work offline. It's a template: vite-plugin-offline.js fills in the version and the
// list of files to precache when building (it isn't used by the dev server).
//
// What's needed to play and create puzzles (play.html and what it loads, except the scanner) is downloaded when the
// worker installs. Everything else (the scanner, other pages) is cached the first time it's loaded, so it also works
// offline after that. Cached files are served from the cache first. Chunk file names aren't content-hashed, so a whole build is cached together (one cache
// per version) and a new version only takes over once every tab using the old one has closed, so a page never mixes
// files from two builds. Cached files never expire on their own (no TTL): they're kept until a newer build replaces
// them, or the browser evicts the site's storage (src/offline/registerServiceWorker.ts asks for it to be persistent).

const VERSION = __VERSION__;
const PRECACHE = __PRECACHE__;

const CACHE_PREFIX = 'slither-offline-';
const CACHE_NAME = `${CACHE_PREFIX}${VERSION}`;

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE_NAME);
      // Bypass the HTTP cache, so we never store a stale copy of an (unhashed) file from an earlier build
      await cache.addAll(PRECACHE.map((path) => new Request(path, { cache: 'reload' })));
    })(),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys.filter((key) => key.startsWith(CACHE_PREFIX) && key !== CACHE_NAME).map((key) => caches.delete(key)),
      );
      await self.clients.claim();
    })(),
  );
});

// Pages are linked without extensions (e.g. `./play`) and with query parameters, so try the variants we precached
const getCandidateURLs = (request) => {
  const url = new URL(request.url);
  url.hash = '';
  const candidates = [url.href];
  if (request.mode === 'navigate') {
    url.search = '';
    if (url.pathname.endsWith('/')) {
      candidates.push(new URL('index.html', url).href);
    } else if (!/\.[a-z0-9]+$/i.test(url.pathname)) {
      candidates.push(`${url.href}.html`);
    }
  }
  return candidates;
};

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) {
    return;
  }

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_NAME);
      for (const candidate of getCandidateURLs(request)) {
        const cached = await cache.match(candidate, { ignoreSearch: request.mode === 'navigate' });
        if (cached) {
          return cached;
        }
      }

      // Not precached (e.g. the developer pages): use the network, keeping a copy for when we're offline
      const response = await fetch(request);
      if (response.ok && response.type === 'basic') {
        await cache.put(request, response.clone());
      }
      return response;
    })(),
  );
});
