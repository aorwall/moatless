import Prism from 'prismjs'
import type { HunkTokens } from 'react-diff-view'

interface Hunk {
  changes: Array<{
    content?: string
    type: string
  }>
}

// Import common language grammars
// Web languages
import 'prismjs/components/prism-markup' // HTML/XML
import 'prismjs/components/prism-javascript'
import 'prismjs/components/prism-typescript'
import 'prismjs/components/prism-jsx'
import 'prismjs/components/prism-tsx'
import 'prismjs/components/prism-css'
import 'prismjs/components/prism-scss'
import 'prismjs/components/prism-less'

// Data formats
import 'prismjs/components/prism-json'
import 'prismjs/components/prism-yaml'
import 'prismjs/components/prism-toml'

// Programming languages
import 'prismjs/components/prism-python'
import 'prismjs/components/prism-java'
import 'prismjs/components/prism-c'
import 'prismjs/components/prism-cpp'
import 'prismjs/components/prism-csharp'
import 'prismjs/components/prism-php'
import 'prismjs/components/prism-ruby'
import 'prismjs/components/prism-go'
import 'prismjs/components/prism-rust'
import 'prismjs/components/prism-swift'
import 'prismjs/components/prism-kotlin'

// Shell and scripting
import 'prismjs/components/prism-bash'
import 'prismjs/components/prism-shell-session'
import 'prismjs/components/prism-powershell'

// Other languages
import 'prismjs/components/prism-sql'
import 'prismjs/components/prism-markdown'
import 'prismjs/components/prism-diff'
import 'prismjs/components/prism-docker'
import 'prismjs/components/prism-makefile'
import 'prismjs/components/prism-graphql'
import 'prismjs/components/prism-nginx'
import 'prismjs/components/prism-http'
import 'prismjs/components/prism-regex'

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
  html: 'markup',
  htm: 'markup',
  xml: 'markup',
  svg: 'markup',
  mathml: 'markup',
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

  // API/Config
  graphql: 'graphql',
  gql: 'graphql',
  nginx: 'nginx',
  conf: 'nginx',
  http: 'http',

  // Special files
  gitignore: 'bash',
  dockerignore: 'bash',
  env: 'bash',
  regex: 'regex',
  htaccess: 'bash',
}

// Shebang patterns for language detection
const shebangMap: Record<string, string> = {
  python: 'python',
  python2: 'python',
  python3: 'python',
  node: 'javascript',
  bash: 'bash',
  sh: 'bash',
  zsh: 'bash',
  ruby: 'ruby',
  perl: 'perl',
  php: 'php',
}

function getLanguageFromShebang(firstLine: string): string | null {
  if (!firstLine.startsWith('#!')) return null

  const shebang = firstLine.toLowerCase()
  for (const [key, lang] of Object.entries(shebangMap)) {
    if (shebang.includes(key)) {
      return lang
    }
  }

  return null
}

function getLanguageFromPath(filePath: string | undefined, firstLine?: string): string {
  if (!filePath) return 'text'

  // Check for shebang first
  if (firstLine) {
    const shebangLang = getLanguageFromShebang(firstLine)
    if (shebangLang) return shebangLang
  }

  // Special handling for specific file names
  const filename = filePath.split('/').pop()?.toLowerCase()
  if (filename) {
    // Common configuration files
    if (filename === 'dockerfile' || filename.startsWith('dockerfile.')) return 'docker'
    if (filename === 'makefile' || filename.startsWith('makefile.')) return 'makefile'
    if (filename === 'gemfile') return 'ruby'
    if (filename === 'rakefile') return 'ruby'
    if (filename === 'package.json' || filename === 'tsconfig.json') return 'json'
    if (filename === '.gitignore' || filename === '.dockerignore') return 'bash'
  }

  const extension = filePath.split('.').pop()?.toLowerCase()
  if (!extension) return 'text'

  return languageMap[extension] || 'text'
}

// Import TokenNode from react-diff-view
import type { TokenNode as DiffTokenNode } from 'react-diff-view'

// Cache for tokenized lines to improve performance
const tokenCache = new Map<string, DiffTokenNode[]>()

// Convert Prism token to react-diff-view token format
function convertPrismToken(token: Prism.Token | string, parentType?: string): DiffTokenNode[] {
  if (typeof token === 'string') {
    return [{ type: 'text', value: token }]
  }

  const tokenType = token.type
  const className = parentType ? `token ${parentType}-${tokenType}` : `token ${tokenType}`

  if (typeof token.content === 'string') {
    // Use a custom type that includes className
    return [
      {
        type: 'prism',
        value: token.content,
        className,
      } as { type: string; value: string; className?: string },
    ]
  }

  if (Array.isArray(token.content)) {
    // Flatten nested tokens while preserving parent context
    return token.content.flatMap((child) => convertPrismToken(child, tokenType))
  }

  // Handle Token object recursively
  if (typeof token.content === 'object' && token.content !== null) {
    return convertPrismToken(token.content as Prism.Token, tokenType)
  }

  // Fallback for unexpected content types
  return [{ type: 'text', value: String(token.content) }]
}

// Tokenize a single line of code
function tokenizeLine(line: string, language: string): DiffTokenNode[] {
  if (!Prism.languages[language] || !line) {
    return [{ type: 'text', value: line }]
  }

  // Create cache key based on line content and language
  const cacheKey = `${language}:${line}`

  // Check cache first
  if (tokenCache.has(cacheKey)) {
    return tokenCache.get(cacheKey)!
  }

  try {
    const tokens = Prism.tokenize(line, Prism.languages[language])
    const result = tokens.flatMap((token) => convertPrismToken(token))

    // Cache the result (limit cache size to prevent memory issues)
    if (tokenCache.size > 10000) {
      // Clear oldest entries when cache gets too large
      const firstKey = tokenCache.keys().next().value
      if (firstKey) tokenCache.delete(firstKey)
    }
    tokenCache.set(cacheKey, result)

    return result
  } catch (error) {
    console.error('Error tokenizing line:', error)
    return [{ type: 'text', value: line }]
  }
}

// Main tokenize function for react-diff-view
export function tokenizeHunks(hunks: Array<Hunk>, filePath?: string): HunkTokens {
  // Get first line content for shebang detection
  let firstLine: string | undefined
  if (hunks.length > 0 && hunks[0].changes.length > 0) {
    const firstChange = hunks[0].changes[0]
    if (firstChange.content) {
      firstLine = firstChange.content
    }
  }

  const language = getLanguageFromPath(filePath, firstLine)
  const allOldLines: DiffTokenNode[][] = []
  const allNewLines: DiffTokenNode[][] = []

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
