import hljs from 'highlight.js/lib/core'

// Register only common languages to keep bundle small
import bash from 'highlight.js/lib/languages/bash'
import css from 'highlight.js/lib/languages/css'
import go from 'highlight.js/lib/languages/go'
import java from 'highlight.js/lib/languages/java'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import markdown from 'highlight.js/lib/languages/markdown'
import python from 'highlight.js/lib/languages/python'
import rust from 'highlight.js/lib/languages/rust'
import sql from 'highlight.js/lib/languages/sql'
import typescript from 'highlight.js/lib/languages/typescript'
import xml from 'highlight.js/lib/languages/xml' // includes HTML
import yaml from 'highlight.js/lib/languages/yaml'

// Register languages
hljs.registerLanguage('javascript', javascript)
hljs.registerLanguage('js', javascript)
hljs.registerLanguage('jsx', javascript)
hljs.registerLanguage('typescript', typescript)
hljs.registerLanguage('ts', typescript)
hljs.registerLanguage('tsx', typescript)
hljs.registerLanguage('python', python)
hljs.registerLanguage('py', python)
hljs.registerLanguage('json', json)
hljs.registerLanguage('css', css)
hljs.registerLanguage('html', xml)
hljs.registerLanguage('xml', xml)
hljs.registerLanguage('bash', bash)
hljs.registerLanguage('sh', bash)
hljs.registerLanguage('shell', bash)
hljs.registerLanguage('yaml', yaml)
hljs.registerLanguage('yml', yaml)
hljs.registerLanguage('markdown', markdown)
hljs.registerLanguage('md', markdown)
hljs.registerLanguage('go', go)
hljs.registerLanguage('rust', rust)
hljs.registerLanguage('rs', rust)
hljs.registerLanguage('java', java)
hljs.registerLanguage('sql', sql)

export interface HighlightToken {
  /** Token type: 'keyword', 'string', 'comment', etc. or 'plain' for unhighlighted text */
  type: string
  content: string
}

/**
 * Highlight a single line of code and return tokens
 * @param code - The code string to highlight
 * @param language - Optional language hint (e.g., 'typescript', 'python')
 * @returns Array of highlight tokens
 */
export function highlightLine(code: string, language?: string): HighlightToken[] {
  if (!code) {
    return [{ type: 'plain', content: code }]
  }

  try {
    const result =
      language && hljs.getLanguage(language)
        ? hljs.highlight(code, { language, ignoreIllegals: true })
        : hljs.highlightAuto(code)

    return parseHljsHtml(result.value)
  } catch {
    return [{ type: 'plain', content: code }]
  }
}

/**
 * Check if a language is supported by the highlighter
 */
export function isLanguageSupported(language: string): boolean {
  return hljs.getLanguage(language) !== undefined
}

/**
 * Parse highlight.js HTML output into tokens, handling nested spans.
 * Input: '<span class="hljs-keyword">const</span> x = <span class="hljs-number">1</span>'
 * Output: [{ type: 'keyword', content: 'const' }, { type: 'plain', content: ' x = ' }, ...]
 */
function parseHljsHtml(html: string): HighlightToken[] {
  const tokens: HighlightToken[] = []
  const typeStack: string[] = []
  const openTagRegex = /^<span class="hljs-([^"]+)">/
  const closeTag = '</span>'
  let i = 0

  while (i < html.length) {
    if (html[i] === '<') {
      // Check for closing </span>
      if (html.startsWith(closeTag, i)) {
        typeStack.pop()
        i += closeTag.length
        continue
      }

      // Check for opening <span class="hljs-...">
      const openMatch = openTagRegex.exec(html.slice(i))
      if (openMatch?.[1]) {
        typeStack.push(openMatch[1])
        i += openMatch[0].length
        continue
      }

      // Stray '<' — emit as text
      const currentType = typeStack.at(-1) ?? 'plain'
      tokens.push({ type: currentType, content: '<' })
      i++
      continue
    }

    // Collect text until next '<'
    let end = html.indexOf('<', i)
    if (end === -1) {
      end = html.length
    }
    const text = html.slice(i, end)
    if (text) {
      const currentType = typeStack.at(-1) ?? 'plain'
      tokens.push({ type: currentType, content: decodeHtml(text) })
    }
    i = end
  }

  return tokens.length > 0 ? tokens : [{ type: 'plain', content: html }]
}

/**
 * Decode HTML entities back to characters
 */
function decodeHtml(html: string): string {
  return html
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&#39;/g, "'")
}

export { default as hljs } from 'highlight.js/lib/core'
