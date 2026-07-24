// Shared iframe capability sets for the app's two preview surfaces.
//
// `IFRAME_ALLOW` is the Permissions-Policy delegated to a preview frame. Bare
// feature names default to the iframe's own (`'src'`) origin, so each capability
// is granted to the previewed content and nothing else. Unknown tokens are
// ignored by the browser.
export const IFRAME_ALLOW = [
  'accelerometer',
  'autoplay',
  'camera',
  'encrypted-media',
  'fullscreen',
  'geolocation',
  'gyroscope',
  'microphone',
  'midi',
  'clipboard-read',
  'clipboard-write',
  'payment',
  'serial',
  'usb',
  'xr-spatial-tracking',
  'screen-wake-lock',
  'magnetometer',
  'picture-in-picture',
  'display-capture',
  'bluetooth',
].join('; ')

// Sandbox tokens shared by both previews. Notably absent: `allow-same-origin`.
// It is only safe when the frame loads a *different* origin than the app; it
// must never be combined with these on a `srcDoc` frame, which inherits our
// origin and would then run untrusted content as the app itself.
const BASE_SANDBOX = [
  'allow-scripts',
  'allow-forms',
  'allow-popups',
  'allow-modals',
  'allow-downloads',
  'allow-pointer-lock',
  'allow-presentation',
  'allow-orientation-lock',
]

// Static preview of an untrusted workspace HTML file via `srcDoc`. Scripts run,
// but in a unique opaque origin (no `allow-same-origin`), so they cannot reach
// the parent app's cookies, storage, or DOM.
export const HTML_PREVIEW_SANDBOX = BASE_SANDBOX.join(' ')

// Live preview of the running app, loaded from a distinct proxy origin. Here
// `allow-same-origin` only lets the app use its own cookies/localStorage/
// IndexedDB — it cannot escape into the parent frame across the origin boundary.
export const LIVE_PREVIEW_SANDBOX = [
  ...BASE_SANDBOX,
  'allow-same-origin',
  'allow-storage-access-by-user-activation',
].join(' ')
