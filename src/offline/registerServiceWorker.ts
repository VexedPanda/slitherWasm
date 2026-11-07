// Registers the service worker (offline/sw.js, generated when building) that lets the site work offline. Only in
// builds, since the dev server doesn't have one.
export const registerServiceWorker = (): void => {
  if (import.meta.env.PROD && 'serviceWorker' in navigator) {
    // Wait until the page has loaded (and had a moment to show the puzzle), since installing a new version downloads
    // everything for offline use, which would otherwise compete with the page's own loading
    const register = () => {
      setTimeout(() => {
        navigator.serviceWorker.register(new URL('sw.js', document.baseURI)).catch((e) => {
          console.warn('Service worker registration failed, so this will not work offline', e);
        });
      }, 1000);
    };
    if (document.readyState === 'complete') {
      register();
    } else {
      window.addEventListener('load', register, { once: true });
    }

    // The offline cache has no expiry of its own (it's kept until a newer build replaces it), but browsers may evict
    // "best-effort" storage under storage pressure or after a while without visits (e.g. Safari after 7 days). Ask
    // for persistent storage so it's kept. Chromium decides silently (e.g. granting it to bookmarked or installed
    // sites), Firefox asks the user once; if it's denied, nothing else changes.
    navigator.storage?.persist?.().catch(() => {});
  }
};
