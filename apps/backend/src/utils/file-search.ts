import { type Dirent, promises as fs } from 'node:fs'
import * as path from 'node:path'
import { glob } from 'glob'
import ignore from 'ignore'

export interface FileSearchRequest {
  query: string
  limit?: number
}

export interface FileSearchResult {
  path: string
  absolutePath: string
  score: number
}

export interface FileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
}

export interface FileSearchResponseLegacy {
  files: string[]
  totalCount: number
  truncated: boolean
}

export interface FileStats {
  size: number
  modified: Date
  isDirectory: boolean
}

export interface ContentMatch {
  line: number
  content: string
  column: number
}

export interface FileContentMatch {
  path: string
  matches: ContentMatch[]
}

export interface ContentSearchResponse {
  files: FileContentMatch[]
  totalMatches: number
}

export interface ListDirectoryOptions {
  recursive?: boolean
  includeHidden?: boolean
  fileOnly?: boolean
}

/**
 * Check if a file is a text file based on its extension or filename
 */
export function isTextFile(filePath: string): boolean {
  const textExtensions = [
    '.txt',
    '.md',
    '.js',
    '.ts',
    '.jsx',
    '.tsx',
    '.json',
    '.xml',
    '.html',
    '.htm',
    '.css',
    '.scss',
    '.sass',
    '.less',
    '.yaml',
    '.yml',
    '.toml',
    '.ini',
    '.conf',
    '.py',
    '.rb',
    '.php',
    '.java',
    '.c',
    '.cpp',
    '.h',
    '.hpp',
    '.cs',
    '.go',
    '.rs',
    '.swift',
    '.kt',
    '.scala',
    '.sh',
    '.bash',
    '.zsh',
    '.fish',
    '.ps1',
    '.sql',
    '.graphql',
    '.gql',
    '.proto',
    '.dockerfile',
    '.gitignore',
    '.env',
    '.log',
    '.csv',
    '.tsv',
    '.svg',
    '.vue',
    '.svelte',
    '.astro',
  ]

  const specialFiles = [
    'makefile',
    'dockerfile',
    'jenkinsfile',
    'vagrantfile',
    'gemfile',
    'podfile',
    'rakefile',
    'package-lock.json',
    'yarn.lock',
    'composer.lock',
  ]

  const ext = path.extname(filePath).toLowerCase()
  const filename = path.basename(filePath).toLowerCase()

  return textExtensions.includes(ext) || specialFiles.includes(filename) || filename.startsWith('.') // Hidden files are usually text
}

/**
 * Get file statistics
 */
export async function getFileStats(filePath: string): Promise<FileStats> {
  const stats = await fs.stat(filePath)
  return {
    size: stats.size,
    modified: stats.mtime,
    isDirectory: stats.isDirectory(),
  }
}

/**
 * Read file content with specified encoding
 */
export async function readFileContent(
  filePath: string,
  encoding: BufferEncoding = 'utf8',
): Promise<string> {
  return await fs.readFile(filePath, encoding)
}

/**
 * List directory contents
 */
export async function listDirectory(
  dirPath: string,
  options: ListDirectoryOptions = {},
): Promise<string[]> {
  const { recursive = false, includeHidden = false, fileOnly = false } = options

  const pattern = recursive ? '**/*' : '*'
  const ignorePatterns = []

  if (!includeHidden) {
    ignorePatterns.push('**/.*', '.*')
  }

  const globOptions = {
    cwd: dirPath,
    ignore: ['**/node_modules/**', '**/.git/**', ...ignorePatterns],
    nodir: fileOnly,
  }

  return await glob(pattern, globOptions)
}

/**
 * Search for content within files
 */
export async function searchFileContents(
  dirPath: string,
  query: string,
  filePattern?: string,
  limit?: number,
): Promise<ContentSearchResponse> {
  try {
    const pattern = filePattern || '**/*'
    const files = await glob(pattern, {
      cwd: dirPath,
      ignore: ['**/node_modules/**', '**/.git/**'],
      nodir: true,
    })

    const results: FileContentMatch[] = []
    let totalMatches = 0
    const queryLower = query.toLowerCase()

    for (const file of files) {
      if (limit && results.length >= limit) {
        break
      }

      if (!isTextFile(file)) {
        continue
      }

      try {
        const filePath = path.join(dirPath, file)
        const content = await fs.readFile(filePath, 'utf8')
        const lines = content.split('\n')
        const matches: ContentMatch[] = []

        for (const [index, line] of lines.entries()) {
          const lineLower = line.toLowerCase()
          const columnIndex = lineLower.indexOf(queryLower)
          if (columnIndex !== -1) {
            matches.push({
              line: index + 1,
              content: line,
              column: columnIndex + 1,
            })
            totalMatches++
          }
        }

        if (matches.length > 0) {
          results.push({
            path: file,
            matches,
          })
        }
      } catch (_error) {
        /* ignore unreadable files */
      }
    }

    return {
      files: results,
      totalMatches,
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Content search failed: ${message}`)
  }
}

/**
 * Load gitignore patterns from repository
 */
async function loadGitignorePatterns(dirPath: string): Promise<ReturnType<typeof ignore>> {
  const ig = ignore()

  // Always ignore .git directory
  ig.add('.git/')

  // Load .gitignore file
  const gitignorePath = path.join(dirPath, '.gitignore')
  try {
    const gitignoreContent = await fs.readFile(gitignorePath, 'utf8')
    ig.add(gitignoreContent)
  } catch (_error) {
    // .gitignore doesn't exist, which is fine
  }

  return ig
}

/**
 * Search for files in a directory based on a query matching file names and paths (Legacy API)
 */
export async function searchFilesInDirectory(
  directory: string,
  query: string,
  limit = 100,
  filePattern?: string,
): Promise<FileSearchResponseLegacy> {
  try {
    const pattern = filePattern || '**/*'
    const files = await glob(pattern, {
      cwd: directory,
      ignore: ['**/node_modules/**', '**/.git/**'],
      nodir: true,
    })

    const queryLower = query.toLowerCase()
    const matchingFiles = files
      .filter((file) => file.toLowerCase().includes(queryLower))
      .slice(0, limit)

    // Also search file contents if they're text files
    const contentMatches: string[] = []
    for (const file of files) {
      if (matchingFiles.length >= limit) {
        break
      }
      if (matchingFiles.includes(file)) {
        continue
      }
      if (!isTextFile(file)) {
        continue
      }

      try {
        const filePath = path.join(directory, file)
        const content = await fs.readFile(filePath, 'utf8')
        if (content.toLowerCase().includes(queryLower)) {
          contentMatches.push(file)
        }
      } catch (_error) {
        // Skip files that can't be read
      }
    }

    const allMatches = [...matchingFiles, ...contentMatches].slice(0, limit)

    return {
      files: allMatches,
      totalCount: allMatches.length,
      truncated: allMatches.length >= limit,
    }
  } catch (_error) {
    return {
      files: [],
      totalCount: 0,
      truncated: false,
    }
  }
}

/**
 * Search for files in a directory based on a query matching file names and paths (New API)
 */
export async function searchFilesInDirectoryNew(
  directory: string,
  request: FileSearchRequest,
): Promise<FileSearchResponse> {
  const limit = request.limit || 100
  const queryLower = request.query.toLowerCase()
  const results: [string, number][] = []

  // Load gitignore patterns
  const gitignorePatterns = await loadGitignorePatterns(directory)

  // Recursively search through directory
  async function searchDir(currentPath: string, relativePath = '') {
    try {
      const entries = await fs.readdir(currentPath, { withFileTypes: true })

      for (const entry of entries) {
        await processEntry(entry, currentPath, relativePath)
      }
    } catch (_error) {
      // Skip directories we can't read
    }
  }

  async function processEntry(entry: Dirent, currentPath: string, relativePath: string) {
    const entryPath = path.join(currentPath, entry.name)
    const entryRelativePath = path.join(relativePath, entry.name)

    // Skip if matches gitignore patterns
    if (gitignorePatterns.ignores(entryRelativePath)) {
      return
    }

    if (entry.isDirectory()) {
      await processDirectory(entryPath, entryRelativePath)
    } else if (entry.isFile()) {
      processFile(entry, entryRelativePath)
    }
  }

  async function processDirectory(entryPath: string, entryRelativePath: string) {
    // Skip directories matching gitignore (with trailing slash)
    if (gitignorePatterns.ignores(`${entryRelativePath}/`)) {
      return
    }
    // Recursively search subdirectory
    await searchDir(entryPath, entryRelativePath)
  }

  function processFile(entry: Dirent, entryRelativePath: string) {
    // Check if query appears in the file path
    const relativePathLower = entryRelativePath.toLowerCase()
    if (!relativePathLower.includes(queryLower)) {
      return
    }

    const score = calculateFileScore(entry.name, entryRelativePath)
    results.push([entryRelativePath, score])
  }

  function calculateFileScore(fileName: string, filePath: string): number {
    let score = 0.0

    // Calculate relevance score
    if (queryLower === fileName.toLowerCase()) {
      // Exact filename match
      score = 10.0
    } else if (fileName.toLowerCase().includes(queryLower)) {
      // Match in filename
      score = 5.0
    } else {
      // Match in directory path
      score = 1.0
    }

    // Bonus points for fewer path components (shorter paths)
    const pathDepth = filePath.split(path.sep).length
    score += (10 - Math.min(pathDepth, 10)) * 0.1

    return score
  }

  await searchDir(directory)

  // Sort by score (descending) and then by path (ascending)
  results.sort((a, b) => {
    if (a[1] !== b[1]) {
      return b[1] - a[1] // Higher score first
    }
    return a[0].localeCompare(b[0]) // Alphabetical order
  })

  // Take top results
  const topResults = results.slice(0, limit).map(([filePath, score]) => ({
    path: filePath,
    absolutePath: path.join(directory, filePath),
    score,
  }))

  return {
    results: topResults,
    totalFound: results.length,
    query: request.query,
  }
}
