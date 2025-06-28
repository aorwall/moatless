import Prism from 'prismjs'
import type { Hunk, HunkTokens } from 'react-diff-view'

// Import common language grammars
import 'prismjs/components/prism-javascript'
import 'prismjs/components/prism-typescript'
import 'prismjs/components/prism-jsx'
import 'prismjs/components/prism-tsx'
import 'prismjs/components/prism-css'
import 'prismjs/components/prism-scss'
import 'prismjs/components/prism-json'
import 'prismjs/components/prism-markdown'
import 'prismjs/components/prism-yaml'
import 'prismjs/components/prism-python'
import 'prismjs/components/prism-java'
import 'prismjs/components/prism-c'
import 'prismjs/components/prism-cpp'
import 'prismjs/components/prism-csharp'
import 'prismjs/components/prism-php'
import 'prismjs/components/prism-ruby'
import 'prismjs/components/prism-go'
import 'prismjs/components/prism-rust'
import 'prismjs/components/prism-sql'
import 'prismjs/components/prism-bash'
import 'prismjs/components/prism-shell-session'
import 'prismjs/components/prism-diff'

// Language mapping based on file extensions
const languageMap: Record<string, string> = {
  // JavaScript/TypeScript
  js: 'javascript',
  jsx: 'jsx',
  ts: 'typescript',
  tsx: 'tsx',
  mjs: 'javascript',
  cjs: 'javascript',

  // Web
  html: 'html',
  htm: 'html',
  xml: 'xml',
  css: 'css',
  scss: 'scss',
  sass: 'scss',
  less: 'less',

  // Data
  json: 'json',
  yaml: 'yaml',
  yml: 'yaml',
  toml: 'toml',

  // Programming languages
  py: 'python',
  java: 'java',
  c: 'c',
  cpp: 'cpp',
  cc: 'cpp',
  cxx: 'cpp',
  cs: 'csharp',
  php: 'php',
  rb: 'ruby',
  go: 'go',
  rs: 'rust',
  swift: 'swift',
  kt: 'kotlin',

  // Scripting
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  fish: 'bash',
  ps1: 'powershell',

  // Database
  sql: 'sql',

  // Documentation
  md: 'markdown',
  mdx: 'markdown',

  // Configuration
  dockerfile: 'docker',
  Dockerfile: 'docker',
  makefile: 'makefile',
  Makefile: 'makefile',

  // Others
  diff: 'diff',
  patch: 'diff',
}

function getLanguageFromPath(filePath: string | undefined): string {
  if (!filePath) return 'text'

  const extension = filePath.split('.').pop()?.toLowerCase()
  if (!extension) return 'text'

  return languageMap[extension] || 'text'
}

// Import TokenNode from react-diff-view
import type { TokenNode as DiffTokenNode } from 'react-diff-view'

// Convert Prism token to react-diff-view token format
function convertPrismToken(token: Prism.Token | string): DiffTokenNode[] {
  if (typeof token === 'string') {
    return [{ type: 'text', value: token }]
  }

  if (typeof token.content === 'string') {
    return [
      {
        type: 'text',
        value: token.content,
        // Add the prism class as a property that can be used by renderToken
        className: `token ${token.type}`,
      } as DiffTokenNode,
    ]
  }

  if (Array.isArray(token.content)) {
    // Flatten nested tokens
    return token.content.flatMap(convertPrismToken)
  }

  // Handle Token object
  return convertPrismToken(token.content as string | Prism.Token)
}

// Tokenize a single line of code
function tokenizeLine(line: string, language: string): DiffTokenNode[] {
  if (!Prism.languages[language] || !line) {
    return [{ type: 'text', value: line }]
  }

  try {
    const tokens = Prism.tokenize(line, Prism.languages[language])
    return tokens.flatMap(convertPrismToken)
  } catch (error) {
    console.error('Error tokenizing line:', error)
    return [{ type: 'text', value: line }]
  }
}

// Main tokenize function for react-diff-view
export function tokenizeHunks(hunks: Hunk[], filePath?: string): HunkTokens {
  const language = getLanguageFromPath(filePath)
  const allOldLines: TokenNode[][] = []
  const allNewLines: TokenNode[][] = []

  // Process each hunk
  for (const hunk of hunks) {
    // Process each change in the hunk
    for (const change of hunk.changes) {
      if (change.type === 'normal') {
        // Normal lines appear in both old and new
        const tokens = tokenizeLine(change.content, language)
        allOldLines.push(tokens)
        allNewLines.push(tokens)
      } else if (change.type === 'delete') {
        // Delete lines only appear in old
        const tokens = tokenizeLine(change.content, language)
        allOldLines.push(tokens)
      } else if (change.type === 'insert') {
        // Insert lines only appear in new
        const tokens = tokenizeLine(change.content, language)
        allNewLines.push(tokens)
      }
    }
  }

  return {
    old: allOldLines,
    new: allNewLines,
  }
}
