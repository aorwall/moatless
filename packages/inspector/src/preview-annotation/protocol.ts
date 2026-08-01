/**
 * The wire vocabulary between an annotation runtime and whatever is hosting it.
 *
 * The same six channels carry the session on both surfaces. Only the pipe
 * differs: Electron sends them over `ipcRenderer`, a browser sends them over
 * `postMessage` between the framed page and the parent document.
 */

export const START_PICK_CHANNEL = 'preview:start-pick'
export const CANCEL_PICK_CHANNEL = 'preview:cancel-pick'
export const ELEMENT_PICKED_CHANNEL = 'preview:element-picked'
export const ANNOTATION_CAPTURED_CHANNEL = 'preview:annotation-captured'
export const ANNOTATION_THEME_CHANNEL = 'preview:annotation-theme'
export const HUMAN_INPUT_CHANNEL = 'preview:human-input'

/**
 * Guest → host, unprompted on load and again in answer to {@link PING_CHANNEL}.
 *
 * A host has no other way to learn whether the page it framed carries the
 * runtime: a cross-origin document cannot be read, and a page that never loaded
 * the script is indistinguishable from one that has not finished loading it.
 * So the guest announces itself, and the host keeps its pick affordance off
 * until it hears this.
 */
export const GUEST_READY_CHANNEL = 'preview:annotation-ready'

/**
 * Host → guest, asking for a {@link GUEST_READY_CHANNEL} answer.
 *
 * Announcing on load is not enough on its own — the host may mount its listener
 * after the page has already loaded, in which case the announcement is long
 * gone. Asking covers that race from the other side.
 */
export const PING_CHANNEL = 'preview:annotation-ping'

/**
 * Marks a `postMessage` as ours.
 *
 * A page shares its `message` event with every frame, extension, and library
 * that also uses `postMessage`, so both ends filter on this before reading
 * anything else out of the payload.
 */
export const PREVIEW_ANNOTATION_MESSAGE = 't3code:preview-annotation'

export interface PreviewAnnotationMessage {
  readonly source: typeof PREVIEW_ANNOTATION_MESSAGE
  readonly channel: string
  readonly args: readonly unknown[]
}

/** The envelope if this is one of ours, `null` for the traffic that isn't. */
export function readPreviewAnnotationMessage(data: unknown): PreviewAnnotationMessage | null {
  if (typeof data !== 'object' || data === null) {
    return null
  }
  const candidate = data as Partial<PreviewAnnotationMessage>
  if (candidate.source !== PREVIEW_ANNOTATION_MESSAGE) {
    return null
  }
  if (typeof candidate.channel !== 'string') {
    return null
  }
  if (!Array.isArray(candidate.args)) {
    return null
  }
  return { source: PREVIEW_ANNOTATION_MESSAGE, channel: candidate.channel, args: candidate.args }
}

export function previewAnnotationMessage(
  channel: string,
  args: readonly unknown[],
): PreviewAnnotationMessage {
  return { source: PREVIEW_ANNOTATION_MESSAGE, channel, args }
}
