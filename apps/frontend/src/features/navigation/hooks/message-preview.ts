export const MAX_PREVIEW_LENGTH = 140

/** Collapse whitespace and clamp text to a single sidebar-friendly line. */
export function formatPreview(text: string): string {
  const collapsed = text.replace(/\s+/g, ' ').trim()
  return collapsed.length > MAX_PREVIEW_LENGTH
    ? `${collapsed.slice(0, MAX_PREVIEW_LENGTH)}…`
    : collapsed
}
