/**
 * The shapes the annotation runtime produces.
 *
 * These are plain interfaces on purpose. A host validates them however it
 * likes — T3 Code wraps them in Effect schemas for its IPC boundary — but the
 * runtime that builds them has no validator and no framework, because it runs
 * inside somebody else's page.
 */

/** Theme tokens a host passes down so the overlay matches its own chrome. */
export interface PreviewAnnotationTheme {
  colorScheme: 'light' | 'dark'
  radius: string
  background: string
  foreground: string
  popover: string
  popoverForeground: string
  primary: string
  primaryForeground: string
  muted: string
  mutedForeground: string
  accent: string
  accentForeground: string
  border: string
  input: string
  ring: string
  fontSans: string
  fontMono: string
}

/**
 * One stack frame from react-grab's `getElementContext`. The source file and
 * line are what let a coding agent jump to the JSX that produced the node.
 */
export interface PickedElementStackFrame {
  functionName: string | null
  fileName: string | null
  lineNumber: number | null
  columnNumber: number | null
}

/**
 * A picked element.
 *
 * Every field is best-effort: a page with no React fiber tree still yields a
 * usable payload (selector plus HTML preview), just without component or source
 * attribution.
 */
export interface PickedElementPayload {
  /** URL of the page the element was picked on. */
  pageUrl: string
  /** `<title>` of that page, when it has one. */
  pageTitle: string | null
  /** Lowercase tag name, e.g. `"button"`. */
  tagName: string
  /** CSS selector that resolves back to the element after a re-render. */
  selector: string | null
  /** Truncated outer-HTML preview (react-grab's `htmlPreview`). */
  htmlPreview: string
  /** Nearest React component display name, or null when unavailable. */
  componentName: string | null
  /** First source-mapped frame — the file and line of the JSX. */
  source: PickedElementStackFrame | null
  /** Full owner stack; can be empty. */
  stack: readonly PickedElementStackFrame[]
  /** Author CSS only, UA defaults stripped (react-grab's `styles`). */
  styles: string
  /** Wall-clock pick time, ISO-8601. */
  pickedAt: string
}

export interface PreviewAnnotationRect {
  x: number
  y: number
  width: number
  height: number
}

export interface PreviewAnnotationPoint {
  x: number
  y: number
}

export interface PreviewAnnotationElementTarget {
  id: string
  element: PickedElementPayload
  rect: PreviewAnnotationRect
}

export interface PreviewAnnotationRegionTarget {
  id: string
  rect: PreviewAnnotationRect
}

export interface PreviewAnnotationStrokeTarget {
  id: string
  color: string
  width: number
  points: readonly PreviewAnnotationPoint[]
  bounds: PreviewAnnotationRect
}

export interface PreviewAnnotationStyleChange {
  targetId: string
  selector: string | null
  property: string
  previousValue: string
  value: string
}

export interface PreviewAnnotationScreenshot {
  dataUrl: string
  width: number
  height: number
  cropRect: PreviewAnnotationRect
}

/**
 * A submitted annotation. One may reference several DOM elements, freeform
 * regions and ink strokes at once. The screenshot is added by the host after
 * the guest submits the structured draft — the guest cannot capture its own
 * page.
 */
export interface PreviewAnnotationPayload {
  id: string
  pageUrl: string
  pageTitle: string | null
  comment: string
  elements: readonly PreviewAnnotationElementTarget[]
  regions: readonly PreviewAnnotationRegionTarget[]
  strokes: readonly PreviewAnnotationStrokeTarget[]
  styleChanges: readonly PreviewAnnotationStyleChange[]
  screenshot: PreviewAnnotationScreenshot | null
  createdAt: string
}
