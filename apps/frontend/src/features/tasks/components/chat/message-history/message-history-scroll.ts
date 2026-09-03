/**
 * Pure scroll-position logic for the message history.
 *
 * Stick-to-bottom, the previous-page fetch trigger with its re-arm workaround,
 * and the scroll-down button all derive from the scroll container's geometry,
 * and getting that arithmetic subtly wrong races under SSE-driven re-renders.
 * Extracting the decisions here makes them independently unit-testable (jsdom
 * can't measure real layout, so the component wires these to real metrics and
 * E2E covers the DOM side) and is the regression net for the planned
 * virtualization migration.
 */

/** How close to the top (in px) before we trigger a previous-page fetch. */
export const FETCH_THRESHOLD = 300
/**
 * How close to the bottom (in px) before we consider the user "at bottom" for
 * stick-to-bottom behaviour and the scroll-down button visibility.
 */
export const AT_BOTTOM_THRESHOLD = 50

/** The geometry of a scroll container, as read from the DOM element. */
export interface ScrollMetrics {
  scrollTop: number
  scrollHeight: number
  clientHeight: number
}

/** Distance (px) from the current scroll position to the bottom of the content. */
export function distanceFromBottom(metrics: ScrollMetrics): number {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight
}

/** Whether the viewport is within `threshold` px of the bottom. */
export function isAtBottom(
  metrics: ScrollMetrics,
  threshold: number = AT_BOTTOM_THRESHOLD,
): boolean {
  return distanceFromBottom(metrics) <= threshold
}

/** The scroll-down button shows precisely when the user is not at the bottom. */
export function shouldShowScrollDownButton(atBottom: boolean): boolean {
  return !atBottom
}

/**
 * Stick-to-bottom decision. Uses the metrics captured at the END of the
 * previous commit so the choice reflects where the user was BEFORE this
 * commit's data change — the ref snapshot that dodges the queued-scroll-event
 * race. Snaps to the bottom only if the user was already there.
 */
export function shouldStickToBottom(
  prev: ScrollMetrics,
  threshold: number = AT_BOTTOM_THRESHOLD,
): boolean {
  return distanceFromBottom(prev) <= threshold
}

/**
 * The previous-page fetch trigger re-arms only once the user has scrolled back
 * past `FETCH_THRESHOLD`, so the anchor-restore (or a short prepended page that
 * lands the user near the top) can't retrigger a fetch with no user intent.
 */
export function shouldRearmFetch(
  scrollTop: number,
  fetchThreshold: number = FETCH_THRESHOLD,
): boolean {
  return scrollTop > fetchThreshold
}

export interface FetchPreviousPageState {
  scrollTop: number
  /** Re-armed by a deliberate scroll past the threshold; disarmed after a fetch. */
  armed: boolean
  hasPreviousPage: boolean
  isFetchingPreviousPage: boolean
  /** Whether a `fetchPreviousPage` callback is available. */
  canFetch: boolean
  /** Whether a prepend anchor is already pending (a fetch is mid-flight). */
  anchorPending: boolean
}

/**
 * Whether an upward scroll should kick off a previous-page fetch. Mirrors the
 * guarded condition in the scroll handler: near the top, armed, more pages
 * available, not already fetching, a callback exists, and no anchor is pending.
 */
export function shouldFetchPreviousPage(
  state: FetchPreviousPageState,
  fetchThreshold: number = FETCH_THRESHOLD,
): boolean {
  return (
    state.scrollTop <= fetchThreshold &&
    state.armed &&
    state.hasPreviousPage &&
    !state.isFetchingPreviousPage &&
    state.canFetch &&
    !state.anchorPending
  )
}

/**
 * After a previous-page fetch commits, the content grows at the top. Restore
 * the reading anchor to `top + (newScrollHeight - oldHeight)` so the user's
 * position stays pinned to the same items. Returns the new `scrollTop`, or
 * `null` when the content did not grow (nothing to restore).
 */
export function computeAnchorRestore(
  newScrollHeight: number,
  anchor: { top: number; height: number },
): number | null {
  const delta = newScrollHeight - anchor.height
  if (delta <= 0) {
    return null
  }
  return anchor.top + delta
}
