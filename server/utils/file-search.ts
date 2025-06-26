import { promises as fs } from 'fs'
import ignore from 'ignore'
import * as path from 'path'

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
  } catch (error) {
    // .gitignore doesn't exist, which is fine
  }

  return ig
}

/**
 * Search for files in a directory based on a query matching file names and paths
 */
export async function searchFilesInDirectory(
  directory: string,
  request: FileSearchRequest,
): Promise<FileSearchResponse> {
  const limit = request.limit || 100
  const queryLower = request.query.toLowerCase()
  const results: Array<[string, number]> = []

  // Load gitignore patterns
  const gitignorePatterns = await loadGitignorePatterns(directory)

  // Recursively search through directory
  async function searchDir(currentPath: string, relativePath: string = '') {
    try {
      const entries = await fs.readdir(currentPath, { withFileTypes: true })

      for (const entry of entries) {
        const entryPath = path.join(currentPath, entry.name)
        const entryRelativePath = path.join(relativePath, entry.name)

        // Skip if matches gitignore patterns
        if (gitignorePatterns.ignores(entryRelativePath)) {
          continue
        }

        if (entry.isDirectory()) {
          // Skip directories matching gitignore (with trailing slash)
          if (gitignorePatterns.ignores(entryRelativePath + '/')) {
            continue
          }
          // Recursively search subdirectory
          await searchDir(entryPath, entryRelativePath)
        } else if (entry.isFile()) {
          // Check if query appears in the file path
          const relativePathLower = entryRelativePath.toLowerCase()
          if (relativePathLower.includes(queryLower)) {
            let score = 0.0

            // Calculate relevance score
            if (queryLower === entry.name.toLowerCase()) {
              // Exact filename match
              score = 10.0
            } else if (entry.name.toLowerCase().includes(queryLower)) {
              // Match in filename
              score = 5.0
            } else {
              // Match in directory path
              score = 1.0
            }

            // Bonus points for fewer path components (shorter paths)
            const pathDepth = entryRelativePath.split(path.sep).length
            score += (10 - Math.min(pathDepth, 10)) * 0.1

            results.push([entryRelativePath, score])
          }
        }
      }
    } catch (error) {
      // Skip directories we can't read
    }
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
