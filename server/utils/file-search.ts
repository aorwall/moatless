import { promises as fs } from 'fs'
import { glob } from 'glob'
import path from 'path'
import type { FileSearchResponse } from '../../shared/types/index.js'

const DEFAULT_IGNORE_PATTERNS = [
  '**/node_modules/**',
  '**/.git/**',
  '**/dist/**',
  '**/build/**',
  '**/.next/**',
  '**/coverage/**',
  '**/*.min.js',
  '**/*.map',
  '**/package-lock.json',
  '**/yarn.lock',
  '**/pnpm-lock.yaml',
]

export async function searchFilesInDirectory(
  dirPath: string,
  query: string,
  maxResults: number = 100,
  filePattern?: string,
): Promise<FileSearchResponse> {
  try {
    // Get all files matching the pattern
    const pattern = filePattern || '**/*'
    const files = await glob(pattern, {
      cwd: dirPath,
      ignore: DEFAULT_IGNORE_PATTERNS,
      nodir: true,
      absolute: false,
      maxDepth: 10,
    })

    // Filter files by query
    const matchingFiles: string[] = []
    const queryLower = query.toLowerCase()

    for (const file of files) {
      // Check if filename contains query
      if (file.toLowerCase().includes(queryLower)) {
        matchingFiles.push(file)
        if (matchingFiles.length >= maxResults) {
          break
        }
        continue
      }

      // Check file content if it's a text file
      if (isTextFile(file) && matchingFiles.length < maxResults) {
        try {
          const content = await fs.readFile(path.join(dirPath, file), 'utf8')
          if (content.toLowerCase().includes(queryLower)) {
            matchingFiles.push(file)
          }
        } catch {
          // Skip files that can't be read
        }
      }

      if (matchingFiles.length >= maxResults) {
        break
      }
    }

    return {
      files: matchingFiles,
      totalCount: matchingFiles.length,
      truncated: matchingFiles.length >= maxResults,
    }
  } catch (error) {
    return {
      files: [],
      totalCount: 0,
      truncated: false,
    }
  }
}

export function isTextFile(filePath: string): boolean {
  const textExtensions = [
    '.js',
    '.jsx',
    '.ts',
    '.tsx',
    '.json',
    '.md',
    '.txt',
    '.py',
    '.java',
    '.c',
    '.cpp',
    '.h',
    '.hpp',
    '.cs',
    '.go',
    '.rs',
    '.php',
    '.rb',
    '.swift',
    '.kt',
    '.scala',
    '.html',
    '.css',
    '.scss',
    '.sass',
    '.less',
    '.xml',
    '.yaml',
    '.yml',
    '.toml',
    '.ini',
    '.conf',
    '.sh',
    '.bash',
    '.zsh',
    '.fish',
    '.ps1',
    '.bat',
    '.dockerfile',
    '.makefile',
  ]

  const ext = path.extname(filePath).toLowerCase()
  const basename = path.basename(filePath).toLowerCase()

  // Check extension first
  if (ext && textExtensions.includes(ext)) {
    return true
  }

  // Check special filenames without extensions
  const specialFiles = ['makefile', 'dockerfile', 'jenkinsfile', 'rakefile', 'gulpfile']
  if (specialFiles.includes(basename)) {
    return true
  }

  // Check dot files that are commonly text
  const textDotFiles = [
    '.gitignore',
    '.env',
    '.eslintrc',
    '.prettierrc',
    '.babelrc',
    '.editorconfig',
  ]
  return textDotFiles.includes(basename)
}

export async function getFileStats(filePath: string): Promise<{
  size: number
  modified: Date
  isDirectory: boolean
}> {
  const stats = await fs.stat(filePath)
  return {
    size: stats.size,
    modified: stats.mtime,
    isDirectory: stats.isDirectory(),
  }
}

export async function readFileContent(
  filePath: string,
  encoding: BufferEncoding = 'utf8',
): Promise<string> {
  return await fs.readFile(filePath, encoding)
}

export async function listDirectory(
  dirPath: string,
  options: {
    recursive?: boolean
    includeHidden?: boolean
    fileOnly?: boolean
  } = {},
): Promise<string[]> {
  const { recursive = false, includeHidden = false, fileOnly = false } = options

  const pattern = recursive ? '**/*' : '*'
  const ignore = includeHidden ? [] : ['**/.*', '.*']

  return await glob(pattern, {
    cwd: dirPath,
    ignore: [...DEFAULT_IGNORE_PATTERNS, ...ignore],
    nodir: fileOnly,
    absolute: false,
  })
}

export async function searchFileContents(
  directory: string,
  query: string,
  filePattern?: string,
  limit: number = 50,
): Promise<{
  files: Array<{
    path: string
    matches: Array<{
      line: number
      content: string
      column: number
    }>
  }>
  totalMatches: number
}> {
  try {
    // Get list of files to search
    const pattern = filePattern || '**/*'
    const files = await glob(pattern, {
      cwd: directory,
      nodir: true,
      ignore: [
        '**/node_modules/**',
        '**/.git/**',
        '**/dist/**',
        '**/build/**',
        '**/*.min.js',
        '**/*.map',
        '**/*.log',
        '**/coverage/**',
        '**/*.png',
        '**/*.jpg',
        '**/*.gif',
        '**/*.pdf',
        '**/*.zip',
      ],
      absolute: false,
      maxDepth: 10,
    })

    const results: Array<{
      path: string
      matches: Array<{
        line: number
        content: string
        column: number
      }>
    }> = []

    let totalMatches = 0
    const regex = new RegExp(query, 'gi')

    for (const file of files.slice(0, limit * 2)) {
      // Search more files than limit
      try {
        const fullPath = path.join(directory, file)
        const content = await fs.readFile(fullPath, 'utf-8')
        const lines = content.split('\n')

        const matches: Array<{
          line: number
          content: string
          column: number
        }> = []

        for (let i = 0; i < lines.length; i++) {
          const line = lines[i]
          let match
          regex.lastIndex = 0 // Reset regex state

          while ((match = regex.exec(line)) !== null) {
            matches.push({
              line: i + 1,
              content: line.trim(),
              column: match.index + 1,
            })
            totalMatches++
          }
        }

        if (matches.length > 0) {
          results.push({
            path: file,
            matches: matches.slice(0, 10), // Limit matches per file
          })
        }

        if (results.length >= limit) break
      } catch (error) {}
    }

    return {
      files: results,
      totalMatches,
    }
  } catch (error) {
    throw new Error(
      `Content search failed: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}
