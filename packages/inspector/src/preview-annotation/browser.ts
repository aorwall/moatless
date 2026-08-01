/**
 * The browser half: a framed page talking to the document that framed it.
 *
 * Importing this module for its side effect is the whole integration — a
 * previewed app loads it in development and becomes annotatable. Nothing here
 * runs unless the page is actually in a frame, so shipping the import to a
 * standalone page costs a single `window.parent === window` check.
 */

import {
  GUEST_READY_CHANNEL,
  previewAnnotationMessage,
  readPreviewAnnotationMessage,
} from './protocol.js'
import { attachPreviewAnnotationRuntime } from './runtime.js'
import type { AnnotationListener, AnnotationTransport } from './transport.js'

/**
 * A transport over `postMessage` to the framing document.
 *
 * Two rules keep this honest in a page that shares its `message` event with
 * everything else on it. Inbound: only what {@link window.parent} sent, and
 * only what carries our envelope. Outbound: `*` until the host has spoken once,
 * then that host's origin and no other — a picked element names files in the
 * repository, so it goes to the window that asked for it rather than to
 * whichever document happens to be listening.
 */
function createFrameTransport(parent: Window): AnnotationTransport {
  const listeners = new Map<string, Set<AnnotationListener>>()
  let hostOrigin = '*'

  window.addEventListener('message', (event: MessageEvent) => {
    if (event.source !== parent) {
      return
    }
    const message = readPreviewAnnotationMessage(event.data)
    if (message === null) {
      return
    }
    hostOrigin = event.origin
    for (const listener of listeners.get(message.channel) ?? []) {
      listener(...message.args)
    }
  })

  return {
    send: (channel, ...args) => {
      // The announcement has to go out before any host has spoken, and it says
      // nothing beyond "a runtime is here" — so it is the one message allowed
      // to leave unaddressed.
      if (hostOrigin === '*' && channel !== GUEST_READY_CHANNEL) {
        return
      }
      parent.postMessage(previewAnnotationMessage(channel, args), hostOrigin)
    },
    on: (channel, listener) => {
      const existing = listeners.get(channel)
      if (existing) {
        existing.add(listener)
      } else {
        listeners.set(channel, new Set([listener]))
      }
    },
    off: (channel, listener) => {
      listeners.get(channel)?.delete(listener)
    },
  }
}

/**
 * Start the runtime if this page is framed, and report whether it did.
 *
 * A page open in its own tab has no host to annotate for, so there is nothing
 * to start and no failure to report.
 */
export function startPreviewAnnotation(): boolean {
  if (typeof window === 'undefined') {
    return false
  }
  if (window.parent === window) {
    return false
  }
  attachPreviewAnnotationRuntime(createFrameTransport(window.parent))
  return true
}

startPreviewAnnotation()
