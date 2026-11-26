# @moatless/inspector

Runtime instrumentation library for AI-assisted UI development. Injects a client script into a previewed page that talks to the frame hosting it over `postMessage`: route tracking and navigation, a heartbeat, console and error forwarding, and element inspection. T3 Code's preview pane is the host this repository ships for.

## How it works

The package has three sides:

- **Build-time (tagger)** — Babel/AST transformer that annotates every JSX element with `data-component-id` and `data-component-name` attributes so the runtime can identify components from DOM clicks.
- **Runtime (client)** — A browser IIFE (`dist/client.js`) injected into the previewed page. Communicates with the parent via `postMessage`.
- **Annotation runtime** — A second client (`dist/annotation-client.js`) for picking, drawing on and commenting about elements, also usable as ESM by a host that supplies its own transport. See [Element annotation](#element-annotation).

## Server-side injection

A server that proxies the previewed app can inject the client script into HTML responses before they reach the iframe. Nothing in this repository does: the sandbox image bakes the package for the Vite plugin below (`docker/Dockerfile.sandbox-base`). The two utilities are exported from the package root:

```ts
import { getInspectorScript, injectScriptIntoHtml } from '@moatless/inspector'

// Read dist/client.js from disk and substitute the server ID placeholder
const script = getInspectorScript({ serverId: 'my-server-123' })

// Splice a <script> tag before </body> (falls back to </html> or end of string)
const instrumentedHtml = injectScriptIntoHtml(html, script)
```

`serverId` is stamped into the client and echoed back in the `INSPECTOR_SCRIPT_LOADED` message so the parent can identify which server instance the iframe belongs to.

## Component tagging

Add `data-component-id="<path>:<line>:<col>"` and `data-component-name` attributes to every JSX element at build time. Required for the element inspector to map DOM clicks back to source locations.

**Vite:**
```ts
// vite.config.ts
import componentTagger from '@moatless/inspector/vite-tagger'

export default defineConfig({
  plugins: [componentTagger()], // devOnly: true by default
})
```

**webpack / Next.js:**
```js
// next.config.js
module.exports = {
  webpack(config) {
    config.module.rules.push({
      test: /\.(jsx|tsx)$/,
      exclude: /node_modules/,
      use: [{ loader: '@moatless/inspector/webpack' }],
    })
    return config
  },
}
```

**Standalone transformer** (e.g. in a custom build pipeline):
```ts
import { transformComponentTags } from '@moatless/inspector/tagger'

const { code, map } = transformComponentTags(source, filePath, rootDir)
```

## Vite dev-server injection

The supported way in: the project adds the HTML plugin to its Vite config, and the dev server injects both clients:

```ts
import inspectorPlugin from '@moatless/inspector/vite'

export default defineConfig({
  plugins: [inspectorPlugin({ serverId: 'vite-dev' })],
})
```

## Element annotation

A second client, `dist/annotation-client.js`, lets a host pick, draw on and
comment about elements in the previewed page — richer than the element
inspector, and it needs no tagger, because it resolves component sources by
walking React fibers at runtime.

The Vite plugin injects it alongside the inspector client, so a project already
calling `inspectorPlugin()` gets it with no config change. It is roughly 80 kB
of inlined script; a project that only needs the basic client
can leave it out with `inspectorPlugin({ annotation: false })`.

The two clients do not know about each other. Each talks to whatever framed the
page over `postMessage` and ignores traffic it does not recognise — the
inspector filters on `source: 'preview-iframe'`, the annotation client on
`source: 't3code:preview-annotation'` — so the one whose host is on the other
side of the frame stays idle.

### Hosting it yourself

The runtime is surface-agnostic: it needs a pipe, not a browser. `postMessage`
is one; T3 Code's desktop app supplies an Electron `ipcRenderer` pipe instead
and runs the same code in a `<webview>` preload.

```ts
import { attachPreviewAnnotationRuntime } from '@moatless/inspector/preview-annotation/runtime'
import type { AnnotationTransport } from '@moatless/inspector/preview-annotation/transport'

attachPreviewAnnotationRuntime({
  send: (channel, ...args) => ipcRenderer.send(channel, ...args),
  on: (channel, listener) => ipcRenderer.on(channel, wrap(listener)),
  off: (channel, listener) => ipcRenderer.off(channel, wrap(listener)),
} satisfies AnnotationTransport)
```

The host side of the conversation needs the channel names and the envelope
helpers, and the shapes that come back:

```ts
import {
  START_PICK_CHANNEL,
  CANCEL_PICK_CHANNEL,
  ELEMENT_PICKED_CHANNEL,
  GUEST_READY_CHANNEL,
  PING_CHANNEL,
  previewAnnotationMessage,
  readPreviewAnnotationMessage,
} from '@moatless/inspector/preview-annotation/protocol'
import type { PreviewAnnotationPayload } from '@moatless/inspector/preview-annotation/types'
```

Both are re-exported from `@moatless/inspector/preview-annotation` for
convenience. The types are plain interfaces with no validator attached — a host
that needs one (T3 Code wraps them in Effect schemas at its IPC boundary) brings
its own.

A guest announces itself on `GUEST_READY_CHANNEL` when its script runs, and
answers `PING_CHANNEL` with the same, so a host that mounted late can still
find it. Until a host hears one of those, it has no way to know whether the page
it framed carries the runtime at all.

### Editing the overlay styles

`src/preview-annotation/styles.generated.ts` is Tailwind output, not source.
After changing `src/preview-annotation/annotation.css` or any class name in
`runtime.ts`, regenerate it:

```bash
bun run build:css
```

## In-iframe API: `window.__moatless`

The injected client exposes a global so apps running inside the preview iframe can communicate with the parent frame programmatically. The global is set during bootstrap (after DOM ready), so check for its existence before calling.

### Detecting inspector presence

```ts
if (window.__moatless) {
  // Inspector script is loaded
}
```

### `window.__moatless.isAvailable()`

Returns `true` if the page has any elements with `data-component-id` attributes — meaning components were tagged at build time and element inspection is functional.

```ts
if (window.__moatless?.isAvailable()) {
  // Safe to show "click to inspect" UI
}
```

### `window.__moatless.isSelectorActive()`

Returns `true` if the parent has switched the element selector (click-to-inspect) on with `TOGGLE_SELECTOR`. T3 Code never sends it, so under T3 Code this stays `false`.

```ts
if (window.__moatless?.isSelectorActive()) {
  // Selector is on — user can click elements to pick them
}
```

### `window.__moatless.toggleChat()`

Sends `TOGGLE_CHAT` to the parent, asking it to collapse or expand its chat panel. T3 Code does not act on it.

```ts
window.__moatless?.toggleChat()
```

### Combined usage example

```ts
const m = window.__moatless
if (m) {
  console.log('Inspector loaded')
  console.log('Elements tagged:', m.isAvailable())
  console.log('Selector active:', m.isSelectorActive())
}
```

## Other features

All of the following run automatically once the client script is injected — no configuration required.

| Feature | Description |
|---|---|
| **Console forwarding** | `console.log/warn/error/info/debug` are intercepted and forwarded to the parent as `console` messages. |
| **Error forwarding** | Uncaught `error` and `unhandledrejection` events are forwarded to the parent. |
| **Element inspector** | Clicking an element (when the inspector is active) walks the DOM to find the nearest `data-component-id` and sends `ELEMENT_CLICKED` with source location and element bounds. |
| **Route tracking** | Patches `history.pushState/replaceState` and listens to `popstate`/`hashchange` to send `ROUTE_CHANGE` on every navigation. |
| **Heartbeat** | Sends a `HEARTBEAT` postMessage every 5 seconds so the parent can detect if the script becomes unreachable. |

## postMessage protocol

All messages carry `source: 'preview-iframe'`. The parent filters by this field.

T3 Code reads `INSPECTOR_SCRIPT_LOADED` and `HEARTBEAT`, which mark the inspector ready, and `ROUTE_CHANGE`, and it sends `NAVIGATE`. No host it ships reads or sends the other types; a host of your own can.

**Iframe → parent (outbound):**

| `type` | Payload | When |
|---|---|---|
| `INSPECTOR_SCRIPT_LOADED` | `{ version, serverId }` | On bootstrap |
| `HEARTBEAT` | `{ timestamp }` | Every 5 s |
| `INSPECTOR_AVAILABILITY` | `{ isAvailable, elementCount }` | Reply to `CHECK_AVAILABILITY_REQUEST` |
| `ELEMENT_CLICKED` | `{ componentId, path, line, column, componentName, elementRect }` | User clicks while inspector is active |
| `ROUTE_CHANGE` | `{ path, timestamp }` | Every client-side navigation |
| `console` | `{ level, message, args, stack? }` | Every `console.*` call |
| `TOGGLE_CHAT` | — | `window.__moatless.toggleChat()` call |

**Parent → iframe (inbound):**

| `type` | Effect |
|---|---|
| `TOGGLE_SELECTOR` | Enable/disable hover+click inspection mode |
| `CHECK_AVAILABILITY_REQUEST` | Count `[data-component-id]` elements, reply with `INSPECTOR_AVAILABILITY` |
| `ADD_TO_SELECTION` | Highlight elements matching a `componentId` with a blue dashed overlay |
| `REMOVE_FROM_SELECTION` | Remove highlight for a `componentId` |
| `CLEAR_SELECTION` | Remove all selection overlays |
| `NAVIGATE` | Navigate the iframe to a given path |
