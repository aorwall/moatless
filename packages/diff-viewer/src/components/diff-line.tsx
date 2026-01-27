import type { DiffLineProps, WordChange } from '../types/index.js'
import { highlightLine } from '../utils/syntax-highlight.js'
import { hasWordChanges } from '../utils/word-diff.js'
import { DiffGutter } from './diff-gutter.js'
import { SyntaxToken } from './syntax-token.js'

/**
 * Renders a single diff line with optional word-level highlighting
 * Styled to match GitHub's diff line display
 *
 * Background logic:
 * - Full line changes (no word-level diffs): darker background (--diff-bg-added/deleted)
 * - Partial changes (with word-level diffs): lighter background (--diff-bg-added/deleted-highlight)
 */
export function DiffLine({ line, language }: DiffLineProps) {
  const getContentBgVar = () => {
    const hasChanges = hasWordChanges(line.changes)
    switch (line.type) {
      case 'add':
        // Lines with word-level changes use lighter background, full line additions use darker
        return hasChanges ? 'var(--diff-bg-added-highlight)' : 'var(--diff-bg-added)'
      case 'del':
        // Lines with word-level changes use lighter background, full line deletions use darker
        return hasChanges ? 'var(--diff-bg-deleted-highlight)' : 'var(--diff-bg-deleted)'
      case 'hunk-header':
        return 'var(--diff-bg-hunk-header)'
      default:
        return 'transparent'
    }
  }

  /**
   * Render content with syntax highlighting
   * @param content - The text content to highlight
   */
  const renderHighlightedContent = (content: string) => {
    if (!language) {
      return <span>{content}</span>
    }
    const tokens = highlightLine(content, language)
    return (
      <>
        {tokens.map((token, i) => (
          <SyntaxToken key={`${token.type}-${i}`} token={token} />
        ))}
      </>
    )
  }

  /**
   * Render word-level changes with syntax highlighting
   * Word changes take visual priority, but tokens within each change get highlighted
   */
  const renderWordChanges = (changes: WordChange[]) => {
    return (
      <>
        {changes.map((change, i) => {
          // Use a composite key of type, index, and first chars of value for stability
          const key = `${change.type}-${i}-${change.value.slice(0, 8)}`

          // Get syntax tokens for this change's content
          const content = language ? (
            highlightLine(change.value, language).map((token, j) => (
              <SyntaxToken key={`${key}-${token.type}-${j}`} token={token} />
            ))
          ) : (
            <span>{change.value}</span>
          )

          if (change.type === 'equal') {
            return <span key={key}>{content}</span>
          }

          // Wrap in highlight span for add/del
          const bgColor =
            change.type === 'add' ? 'var(--diff-word-added)' : 'var(--diff-word-deleted)'

          return (
            <span key={key} className="rounded-sm px-0.5" style={{ backgroundColor: bgColor }}>
              {content}
            </span>
          )
        })}
      </>
    )
  }

  const renderContent = () => {
    // Hunk header gets special styling (no syntax highlighting)
    if (line.type === 'hunk-header') {
      return (
        <span style={{ color: 'var(--diff-text-hunk)' }} className="italic">
          {line.content}
        </span>
      )
    }

    // If we have word-level changes, render them with highlighting
    if (line.changes && line.changes.length > 0) {
      return renderWordChanges(line.changes)
    }

    // Apply syntax highlighting to the whole line
    return renderHighlightedContent(line.content)
  }

  // Hunk headers have a simplified display
  if (line.type === 'hunk-header') {
    return (
      <div className="flex min-h-[24px]">
        <div
          className="flex w-[100px] shrink-0 select-none items-center justify-center"
          style={{ backgroundColor: 'var(--diff-gutter-bg)' }}
        >
          <span className="font-mono text-xs" style={{ color: 'var(--diff-text-hunk)' }}>
            ...
          </span>
        </div>
        <div
          className="flex-1 overflow-x-auto px-3 py-0.5"
          style={{ backgroundColor: getContentBgVar() }}
        >
          <pre className="font-mono text-xs">
            <code>{renderContent()}</code>
          </pre>
        </div>
      </div>
    )
  }

  return (
    <div className="flex min-h-[24px]">
      <DiffGutter oldNumber={line.oldNumber} newNumber={line.newNumber} type={line.type} />
      <div
        className="flex-1 overflow-x-auto px-3 py-0.5"
        style={{ backgroundColor: getContentBgVar() }}
      >
        <pre className="font-mono text-xs">
          <code>{renderContent()}</code>
        </pre>
      </div>
    </div>
  )
}
