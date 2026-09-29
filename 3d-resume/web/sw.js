// Offline copy of the 3D resume (installable web app). Network first: online,
// every request goes to the server, so a new version shows on the next visit
// without a prompt; each response is also kept, and served when offline.
const CACHE = "resume";
// Build outputs carry a content hash (resume-3d-<hash>_bg.wasm); only the
// newest of each is kept.
const HASHED = /\/resume-3d-[0-9a-f]+(_bg\.wasm|\.js)$/;

// The other formats the page links to, so they open offline too (one that's
// missing, like the PDF in a local build, is skipped).
const FORMATS = ["plain.html", "Harald_Reingruber_resume.pdf"];

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(
    caches.open(CACHE).then((cache) =>
      Promise.all(FORMATS.map((url) => cache.add(url).catch(() => {}))),
    ),
  );
});
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET" || new URL(request.url).origin !== self.location.origin) {
    return;
  }
  event.respondWith(
    fetch(request)
      .then((response) => {
        if (response.ok) {
          const copy = response.clone();
          event.waitUntil(keep(request, copy));
        }
        return response;
      })
      .catch(() =>
        caches.match(request, { ignoreSearch: request.mode === "navigate" })
          .then((cached) => cached || Response.error()),
      ),
  );
});

async function keep(request, response) {
  const cache = await caches.open(CACHE);
  const hashed = new URL(request.url).pathname.match(HASHED);
  if (hashed) {
    // Drop older builds of the same file.
    for (const old of await cache.keys()) {
      const match = new URL(old.url).pathname.match(HASHED);
      if (match && match[1] === hashed[1] && old.url !== request.url) {
        await cache.delete(old);
      }
    }
  }
  await cache.put(request, response);
}
