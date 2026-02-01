import type { DiffGutterProps } from '../types/diff'

/**
 * Renders the gutter for a diff line (line numbers + change indicator)
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
      }}
    >
      {/* Old line number */}
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
      {/* New line number */}
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
