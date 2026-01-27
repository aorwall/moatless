import type { HighlightToken } from '../utils/syntax-highlight.js'

interface SyntaxTokenProps {
  token: HighlightToken
}

/**
 * Map of highlight.js token types to CSS custom property names
 * Uses CSS variables for theming support
 */
const tokenColorMap: Record<string, string> = {
  // Keywords: const, let, function, class, if, else, return, etc.
  keyword: 'var(--hljs-keyword)',
  // Strings: "hello", 'world', `template`
  string: 'var(--hljs-string)',
  // Numbers: 123, 45.67, 0xFF
  number: 'var(--hljs-number)',
  // Comments: // comment, /* block */
  comment: 'var(--hljs-comment)',
  // Function names
  function: 'var(--hljs-function)',
  'function-name': 'var(--hljs-function)',
  // Class names and type names
  class: 'var(--hljs-class)',
  'class-name': 'var(--hljs-class)',
  title: 'var(--hljs-function)',
  // Variable names
  variable: 'var(--hljs-variable)',
  // Operators: +, -, *, /, =, etc.
  operator: 'var(--hljs-operator)',
  // Punctuation: {, }, [, ], (, ), etc.
  punctuation: 'var(--hljs-punctuation)',
  // Attributes (HTML, JSX)
  attr: 'var(--hljs-attr)',
  attribute: 'var(--hljs-attr)',
  // HTML/XML tag names
  tag: 'var(--hljs-tag)',
  name: 'var(--hljs-tag)',
  // Built-in types and functions
  built_in: 'var(--hljs-built-in)',
  // Literals: true, false, null, undefined
  literal: 'var(--hljs-keyword)',
  // Type annotations
  type: 'var(--hljs-class)',
  // Function parameters
  params: 'var(--hljs-variable)',
  // Regular expressions
  regexp: 'var(--hljs-string)',
  // Template strings
  subst: 'var(--hljs-variable)',
  // Symbols
  symbol: 'var(--hljs-number)',
  // Meta (preprocessor, decorators)
  meta: 'var(--hljs-meta)',
  // Selector (CSS)
  selector: 'var(--hljs-function)',
  // Property names
  property: 'var(--hljs-attr)',
  // Section headers
  section: 'var(--hljs-function)',
}

/**
 * Renders a syntax-highlighted token with appropriate color
 */
export function SyntaxToken({ token }: SyntaxTokenProps) {
  const color = tokenColorMap[token.type]

  if (!color || token.type === 'plain') {
    return <span>{token.content}</span>
  }

  return <span style={{ color }}>{token.content}</span>
}
