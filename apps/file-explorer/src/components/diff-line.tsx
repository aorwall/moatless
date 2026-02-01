import type { DiffLineProps, WordChange } from '../types/diff'
import { highlightLine } from '../utils/syntax-highlight'
import { hasWordChanges } from '../utils/word-diff'
import { DiffGutter } from './diff-gutter'
import { SyntaxToken } from './syntax-token'

/**
 * Renders a single diff line with optional word-level highlighting
 */
export function DiffLine({ line, language }: DiffLineProps) {
  const getContentBgVar = () => {
    const hasChanges = hasWordChanges(line.changes)
    switch (line.type) {
      case 'add':
        return hasChanges ? 'var(--diff-bg-added-highlight)' : 'var(--diff-bg-added)'
      case 'del':
        return hasChanges ? 'var(--diff-bg-deleted-highlight)' : 'var(--diff-bg-deleted)'
      case 'hunk-header':
        return 'var(--diff-bg-hunk-header)'
      default:
        return 'transparent'
    }
  }

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

  const renderWordChanges = (changes: WordChange[]) => {
    return (
      <>
        {changes.map((change, i) => {
          const key = `${change.type}-${i}-${change.value.slice(0, 8)}`

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
    if (line.type === 'hunk-header') {
      return (
        <span style={{ color: 'var(--diff-text-hunk)' }} className="italic">
          {line.content}
        </span>
      )
    }

    if (line.changes && line.changes.length > 0) {
      return renderWordChanges(line.changes)
    }

    return renderHighlightedContent(line.content)
  }

  if (line.type === 'hunk-header') {
    return (
      <div className="flex">
        <div
          className="flex w-[100px] shrink-0 select-none items-center justify-center"
          style={{ backgroundColor: 'var(--diff-gutter-bg)' }}
        >
          <span className="font-mono text-xs" style={{ color: 'var(--diff-text-hunk)' }}>
            ...
          </span>
        </div>
        <div
          className="flex-1 px-3"
          style={{ backgroundColor: getContentBgVar() }}
        >
          <pre className="m-0 font-mono text-xs leading-5">
            <code>{renderContent()}</code>
          </pre>
        </div>
      </div>
    )
  }

  return (
    <div className="flex">
      <DiffGutter oldNumber={line.oldNumber} newNumber={line.newNumber} type={line.type} />
      <div
        className="flex-1 px-3"
        style={{ backgroundColor: getContentBgVar() }}
      >
        <pre className="m-0 font-mono text-xs leading-5">
          <code>{renderContent()}</code>
        </pre>
      </div>
    </div>
  )
}
