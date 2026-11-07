// Registers the service worker (offline/sw.js, generated when building) that lets the site work offline. Only in
// builds, since the dev server doesn't have one.
export const registerServiceWorker = (): void => {
  if (import.meta.env.PROD && 'serviceWorker' in navigator) {
    navigator.serviceWorker.register(new URL('sw.js', document.baseURI)).catch((e) => {
      console.warn('Service worker registration failed, so this will not work offline', e);
    });
  }
};
