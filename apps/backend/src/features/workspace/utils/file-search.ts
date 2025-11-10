import { type Dirent, promises as fs } from 'node:fs'
import * as path from 'node:path'
import ignore from 'ignore'

interface FileSearchRequest {
  query: string
  limit?: number
}

interface FileSearchResult {
  path: string
  absolutePath: string
  score: number
}

interface FileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
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
