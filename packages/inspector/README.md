# @moatless/inspector

Runtime instrumentation library for AI-assisted UI development. Injects a client script into preview iframes that enables bidirectional communication with the parent frame: element inspection, console forwarding, route tracking, and chat panel control.

## How it works

The package has two sides:

- **Build-time (tagger)** — Babel/AST transformer that annotates every JSX element with `data-component-id` and `data-component-name` attributes so the runtime can identify components from DOM clicks.
- **Runtime (client)** — A browser IIFE (`dist/client.js`) injected into the preview iframe. Communicates with the parent via `postMessage`.

## Server-side injection (primary use)

The preview-server proxy injects the client script into HTML responses before they reach the iframe. Use the two utilities exported from the package root:

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

For injecting the client directly via the Vite dev server (instead of a proxy), use the HTML plugin:

```ts
import inspectorPlugin from '@moatless/inspector/vite'

export default defineConfig({
  plugins: [inspectorPlugin({ serverId: 'vite-dev' })],
})
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

Returns `true` if the element selector (click-to-inspect) mode is currently active in the parent frame.

```ts
if (window.__moatless?.isSelectorActive()) {
  // Selector is on — user can click elements to pick them
}
```

### `window.__moatless.toggleChat()`

Collapses or expands the chat panel in the parent (kiosk mode). Useful for embedding a "show/hide assistant" button inside the preview app itself.

```ts
window.__moatless?.toggleChat()
```

The parent listens for the `TOGGLE_CHAT` postMessage and drives the resizable chat panel accordingly.

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
| **Console forwarding** | `console.log/warn/error/info/debug` are intercepted and forwarded to the parent as `console` messages, shown in the Console tab. |
| **Error forwarding** | Uncaught `error` and `unhandledrejection` events are forwarded to the parent. |
| **Element inspector** | Clicking an element (when the inspector is active) walks the DOM to find the nearest `data-component-id` and sends `ELEMENT_CLICKED` with source location and element bounds. |
| **Route tracking** | Patches `history.pushState/replaceState` and listens to `popstate`/`hashchange` to send `ROUTE_CHANGE` on every navigation. |
| **Heartbeat** | Sends a `HEARTBEAT` postMessage every 5 seconds so the parent can detect if the script becomes unreachable. |

## postMessage protocol

All messages carry `source: 'preview-iframe'`. The parent filters by this field.

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
