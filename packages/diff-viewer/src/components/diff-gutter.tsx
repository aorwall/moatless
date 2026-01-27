import type { DiffGutterProps } from '../types/index.js'

/**
 * Renders the gutter for a diff line (line numbers + change indicator)
 * Styled to match GitHub's diff gutter exactly:
 * - Two line number columns (old + new) for unified diff view
 * - Line numbers are right-aligned with min-width: 32px
 * - Change indicator (+/-) after line numbers
 * - Gutter background colored for add/del lines
 * - Font: 12px, line-height: 20px, height: 20px
 */
export function DiffGutter({ oldNumber, newNumber, type }: DiffGutterProps) {
  const getIndicator = () => {
    switch (type) {
      case 'add':
        return '+'
      case 'del':
        return '-'
      default:
        return ''
    }
  }

  const getIndicatorColorVar = () => {
    switch (type) {
      case 'add':
        return 'var(--diff-text-added)'
      case 'del':
        return 'var(--diff-text-deleted)'
      default:
        return 'var(--diff-line-number)'
    }
  }

  const getGutterBgVar = () => {
    switch (type) {
      case 'add':
        return 'var(--diff-gutter-bg-added)'
      case 'del':
        return 'var(--diff-gutter-bg-deleted)'
      case 'hunk-header':
        return 'var(--diff-gutter-bg-hunk)'
      default:
        return 'var(--diff-gutter-bg)'
    }
  }

  // GitHub uses 'Roboto Mono' font family
  const fontStyle = {
    fontFamily: "'Roboto Mono', monospace",
    fontSize: '12px',
    lineHeight: '20px',
  }

  return (
    <div
      className="diff-gutter flex shrink-0 select-none items-center"
      style={{
        backgroundColor: getGutterBgVar(),
        height: '20px',
      }}
    >
      {/* Old line number - right aligned, min-width 32px */}
      <div
        className="flex items-center justify-end pr-1"
        style={{
          minWidth: '32px',
          color: 'var(--diff-line-number)',
          ...fontStyle,
        }}
      >
        {type !== 'add' && oldNumber !== undefined ? oldNumber : ''}
      </div>
      {/* New line number - right aligned, min-width 32px */}
      <div
        className="flex items-center justify-end pr-1"
        style={{
          minWidth: '32px',
          color: 'var(--diff-line-number)',
          ...fontStyle,
        }}
      >
        {type !== 'del' && newNumber !== undefined ? newNumber : ''}
      </div>
      {/* Change indicator (+/-) */}
      <div
        className="flex items-center justify-center"
        style={{
          width: '20px',
          color: getIndicatorColorVar(),
          ...fontStyle,
        }}
      >
        {getIndicator()}
      </div>
    </div>
  )
}
