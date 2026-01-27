import type { HiddenChunkProps } from '../types/index.js'

/** Lines to expand at a time */
const EXPAND_STEP = 20

/**
 * GitHub-style expand icon (two chevrons with horizontal line)
 * Used for "expand all" button in middle position
 */
function ExpandAllIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      xmlns="http://www.w3.org/2000/svg"
      className="h-4 w-4"
      fill="currentColor"
      aria-hidden="true"
    >
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M10.8536 5.85363C10.6583 6.04889 10.3417 6.04889 10.1464 5.85363L8 3.70718L5.85355 5.85363C5.65829 6.04889 5.34171 6.04889 5.14645 5.85363C4.95118 5.65837 4.95118 5.34178 5.14645 5.14652L7.64645 2.64652L8 2.29297L8.35355 2.64652L10.8536 5.14652C11.0488 5.34178 11.0488 5.65837 10.8536 5.85363ZM10.8536 10.1465C10.6583 9.95126 10.3417 9.95126 10.1464 10.1465L8 12.293L5.85355 10.1465C5.65829 9.95126 5.34171 9.95126 5.14645 10.1465C4.95118 10.3418 4.95118 10.6584 5.14645 10.8536L7.64645 13.3536L8 13.7072L8.35355 13.3536L10.8536 10.8536C11.0488 10.6584 11.0488 10.3418 10.8536 10.1465ZM3 7.50008C2.72386 7.50008 2.5 7.72393 2.5 8.00008C2.5 8.27622 2.72386 8.50008 3 8.50008L13 8.50007C13.2761 8.50007 13.5 8.27622 13.5 8.00007C13.5 7.72393 13.2761 7.50007 13 7.50007L3 7.50008Z"
      />
    </svg>
  )
}

/**
 * Expand down icon - for expanding lines from top (at start of file)
 * Arrow pointing down with horizontal line at top
 */
function ExpandDownIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      xmlns="http://www.w3.org/2000/svg"
      className="h-4 w-4"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M8 6a.5.5 0 0 0-.5.5v4.293L5.854 9.146a.5.5 0 1 0-.708.707l2.497 2.497.007.007.35.35.354-.354 2.5-2.5a.5.5 0 0 0-.708-.707L8.5 10.793V6.5A.5.5 0 0 0 8 6m5.5-3a.5.5 0 0 1 0 1h-11a.5.5 0 1 1 0-1z" />
    </svg>
  )
}

/**
 * Expand up icon - for expanding lines from bottom (at end of file)
 * Arrow pointing up with horizontal line at bottom
 */
function ExpandUpIcon() {
  return (
    <svg
      viewBox="0 0 16 16"
      xmlns="http://www.w3.org/2000/svg"
      className="h-4 w-4"
      fill="currentColor"
      aria-hidden="true"
    >
      <path d="M8 10a.5.5 0 0 1-.5-.5V5.207L5.854 6.854a.5.5 0 1 1-.708-.708l2.497-2.496.007-.007.35-.35.354.353 2.5 2.5a.5.5 0 0 1-.708.708L8.5 5.207V9.5a.5.5 0 0 1-.5.5m5.5 3a.5.5 0 0 0 0-1h-11a.5.5 0 0 0 0 1z" />
    </svg>
  )
}

/**
 * Renders an expandable placeholder for hidden lines
 * Shows different buttons based on position:
 * - start: "↓ N lines" to expand downward from start of file
 * - end: "↑ N lines" to expand upward from end of file
 * - middle: "↕ All N lines" to expand all at once (between changes)
 */
export function HiddenChunk({
  lineCount,
  position = 'middle',
  revealedFromTop = 0,
  revealedFromBottom = 0,
  onExpandTop,
  onExpandBottom,
  onExpandAll,
}: HiddenChunkProps) {
  const remainingLines = lineCount - revealedFromTop - revealedFromBottom

  // Don't render if all lines are revealed
  if (remainingLines <= 0) {
    return null
  }

  const expandCount = Math.min(EXPAND_STEP, remainingLines)

  // Determine which button to show based on position
  let icon: React.ReactNode
  let label: string
  let onClick: (() => void) | undefined

  if (position === 'start') {
    // At start of file: expand downward
    icon = <ExpandDownIcon />
    label = `${expandCount} line${expandCount !== 1 ? 's' : ''}`
    onClick = () => onExpandTop?.(expandCount)
  } else if (position === 'end') {
    // At end of file: expand upward
    icon = <ExpandUpIcon />
    label = `${expandCount} line${expandCount !== 1 ? 's' : ''}`
    onClick = () => onExpandBottom?.(expandCount)
  } else {
    // In the middle: expand all at once
    icon = <ExpandAllIcon />
    label = `All ${remainingLines} line${remainingLines !== 1 ? 's' : ''}`
    onClick = onExpandAll
  }

  return (
    <div className="hidden-chunk">
      {/* Top border spacer */}
      <div className="flex" style={{ height: '4px' }}>
        <div className="flex-1" />
        <div className="flex-1" style={{ borderBottom: '1px solid var(--diff-border)' }} />
      </div>

      {/* Single expand button row */}
      <div
        className="flex items-center justify-between px-2 py-0.5"
        style={{
          backgroundColor: 'var(--diff-bg-hunk-header)',
          minHeight: '24px',
        }}
      >
        <button
          type="button"
          onClick={onClick}
          className="flex items-center gap-1 rounded px-1 py-0.5 text-xs transition-colors hover:bg-black/5"
          style={{ color: 'var(--diff-text-hunk)' }}
        >
          {icon}
          <span>{label}</span>
        </button>
      </div>

      {/* Bottom border spacer */}
      <div className="flex" style={{ height: '4px' }}>
        <div className="flex-1" />
        <div className="flex-1" style={{ borderTop: '1px solid var(--diff-border)' }} />
      </div>
    </div>
  )
}
