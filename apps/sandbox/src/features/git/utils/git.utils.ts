import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { GitChanges, GitSyncStatus } from '@moatless/types'
import { logger } from '../../../shared/logger'

export const execAsync = promisify(exec)

// Diff size limits to prevent maxBuffer overflow and maintain performance
const DIFF_SIZE_LIMITS = {
  MAX_FILE_LINES: 10_000, // Skip files with >10k line changes (matches GitHub standard)
  MAX_TOTAL_FILES: 3_000, // Maximum files in diff
  MAX_BUFFER_MB: 50, // 50MB buffer for git operations (increased from 10MB)
} as const

interface SkippedFile {
  filename: string
  reason: 'too_large' | 'binary'
  additions?: number
  deletions?: number
}

interface GitDiffFile {
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  filename: string
  oldFilename?: string // For renamed files
  additions: number
  deletions: number
  oldContent?: string
  newContent?: string
  isStaged: boolean
}

interface GitDiffResponse {
  files: GitDiffFile[]
  totalChanges: number
  totalAdditions: number
  totalDeletions: number
  diffString: string
  compareBase: string
  skippedFiles?: SkippedFile[]
  warnings?: string[]
}

export async function getCurrentBranch(repoPath: string): Promise<string> {
  try {
    const { stdout } = await execAsync('git branch --show-current', {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get current branch: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function hasUncommittedChanges(repoPath: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync('git status --porcelain', {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim().length > 0
  } catch (error) {
    throw new Error(
      `Failed to check uncommitted changes: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

/**
 * Get untracked files and create diffs for them
 */
async function getUntrackedFilesDiff(repoPath: string): Promise<string> {
  try {
    const { stdout: untrackedOutput } = await execAsync(
      'git ls-files --others --exclude-standard',
      {
        cwd: repoPath,
        timeout: 10000,
      },
    )

    const untrackedFiles = untrackedOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    let untrackedDiffs = ''
    for (const filename of untrackedFiles) {
      if (filename) {
        try {
          // Read the content of untracked file
          const { stdout: fileContent } = await execAsync(`cat "${filename}"`, {
            cwd: repoPath,
            timeout: 5000,
            maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
          })

          // Create a diff-like output for untracked files
          untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
          untrackedDiffs += 'new file mode 100644\n'
          untrackedDiffs += 'index 0000000..0000000\n'
          untrackedDiffs += '--- /dev/null\n'
          untrackedDiffs += `+++ b/${filename}\n`

          // Add file content with + prefix
          const lines = fileContent.split('\n')
          if (lines.length > 0) {
            untrackedDiffs += `@@ -0,0 +1,${lines.length} @@\n`
            for (const line of lines) {
              untrackedDiffs += `+${line}\n`
            }
          }
        } catch {
          // If we can't read the file, just add a header
          untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
          untrackedDiffs += 'new file mode 100644\n'
          untrackedDiffs += 'Binary file\n'
        }
      }
    }

    return untrackedDiffs
  } catch {
    return ''
  }
}

/**
 * Get the best reference to compare against (origin/base > local base > merge base)
 */
export async function getBestCompareReference(
  repoPath: string,
  baseBranch?: string,
): Promise<{ reference: string; type: 'origin' | 'local' | 'merge-base' }> {
  const base = baseBranch || (await detectBaseBranch(repoPath))
  const currentBranch = await getCurrentBranch(repoPath)
  const originBranch = `origin/${base}`

  // First, try to use origin/base if it exists
  try {
    await execAsync(`git rev-parse --verify ${originBranch}`, {
      cwd: repoPath,
      timeout: 5000,
    })
    return { reference: originBranch, type: 'origin' }
  } catch {
    // Origin branch doesn't exist, continue to local options
  }

  // For worktrees, try to get merge base for complete diff
  try {
    const mergeBase = await getMergeBase(repoPath, base, currentBranch)
    return { reference: mergeBase, type: 'merge-base' }
  } catch {
    // Fall back to local base branch
    return { reference: base, type: 'local' }
  }
}

/**
 * Check if origin remote exists and optionally fetch from it
 */
export async function checkAndFetchOrigin(
  repoPath: string,
  baseBranch?: string,
  shouldFetch = true,
): Promise<boolean> {
  try {
    await execAsync('git remote get-url origin', {
      cwd: repoPath,
      timeout: 5000,
    })

    if (shouldFetch) {
      const base = baseBranch || (await detectBaseBranch(repoPath))
      try {
        await execAsync(`git fetch origin ${base}`, {
          cwd: repoPath,
          timeout: 30000,
        })
      } catch (error) {
        logger.warn(
          {
            error: error instanceof Error ? error.message : String(error),
          },
          `Failed to fetch origin/${base}`,
        )
      }
    }

    return true
  } catch {
    return false
  }
}

export async function getGitDiffString(
  repoPath: string,
  baseBranch?: string,
): Promise<[string, string]> {
  try {
    // Check if origin exists but don't fetch (for performance)
    await checkAndFetchOrigin(repoPath, baseBranch, false)

    // Get the best reference to compare against
    const { reference: compareBase, type } = await getBestCompareReference(repoPath, baseBranch)

    logger.debug({ compareBase, type }, 'Using compare reference for git diff')

    const { stdout: diffOutput } = await execAsync(`git diff ${compareBase}`, {
      cwd: repoPath,
      timeout: 30000,
      maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
    })

    // Get untracked files and create diffs for them
    const untrackedDiffs = await getUntrackedFilesDiff(repoPath)

    // Combine regular diff with untracked files diff
    const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')

    return [combinedDiff, compareBase]
  } catch (error) {
    // Try with master as fallback if no baseBranch was specified
    if (!baseBranch) {
      try {
        const { reference: fallbackCompare } = await getBestCompareReference(repoPath, 'master')
        const { stdout: diffOutput } = await execAsync(`git diff ${fallbackCompare}`, {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })

        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')
        return [combinedDiff, fallbackCompare]
      } catch {
        // Final fallback: show only uncommitted changes
        const { stdout: diffOutput } = await execAsync('git diff', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })

        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')
        return [combinedDiff, 'HEAD']
      }
    }
    throw new Error(
      `Failed to get git diff: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function isGitRepository(repoPath: string): Promise<boolean> {
  try {
    await execAsync('git rev-parse --git-dir', {
      cwd: repoPath,
      timeout: 5000,
    })
    return true
  } catch {
    return false
  }
}

export async function getCommitHash(repoPath: string, short = true): Promise<string> {
  try {
    const command = short ? 'git rev-parse --short HEAD' : 'git rev-parse HEAD'
    const { stdout } = await execAsync(command, {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get commit hash: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getGitStatus(repoPath: string): Promise<{
  clean: boolean
  modified: string[]
  untracked: string[]
  staged: string[]
}> {
  try {
    const { stdout } = await execAsync('git status --porcelain', {
      cwd: repoPath,
      timeout: 5000,
    })

    const lines = stdout.split('\n').filter((line) => line.length > 0)
    const modified: string[] = []
    const untracked: string[] = []
    const staged: string[] = []

    for (const line of lines) {
      const status = line.substring(0, 2)
      const filename = line.substring(2).trim()
      if (status[0] !== ' ' && status[0] !== '?') {
        staged.push(filename)
      }
      if (status[1] === 'M' || status[1] === 'D') {
        modified.push(filename)
      }
      if (status === '??') {
        untracked.push(filename)
      }
    }

    return {
      clean: lines.length === 0,
      modified,
      untracked,
      staged,
    }
  } catch (error) {
    throw new Error(
      `Failed to get git status: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function detectBaseBranch(repoPath: string): Promise<string> {
  // Check if main branch exists
  try {
    await execAsync('git rev-parse --verify main', {
      cwd: repoPath,
      timeout: 5000,
    })
    return 'main'
  } catch {
    // Check if master branch exists
    try {
      await execAsync('git rev-parse --verify master', {
        cwd: repoPath,
        timeout: 5000,
      })
      return 'master'
    } catch {
      // Default to main if neither exists
      return 'main'
    }
  }
}

export async function getMergeBase(
  repoPath: string,
  branch1: string,
  branch2: string,
): Promise<string> {
  try {
    const { stdout } = await execAsync(`git merge-base "${branch1}" "${branch2}"`, {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get merge base: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function checkForConflicts(repoPath: string, baseBranch?: string): Promise<boolean> {
  try {
    const base = baseBranch || 'main'

    try {
      const { stdout, stderr } = await execAsync(`git merge-tree --write-tree ${base} HEAD`, {
        cwd: repoPath,
        timeout: 10000,
      })

      return stderr.includes('CONFLICT') || stdout.includes('CONFLICT')
    } catch (error) {
      const err = error as { stderr?: string; stdout?: string }
      if (err.stderr?.includes('CONFLICT') || err.stdout?.includes('CONFLICT')) {
        return true
      }

      if (!baseBranch) {
        try {
          const { stdout, stderr } = await execAsync('git merge-tree --write-tree master HEAD', {
            cwd: repoPath,
            timeout: 10000,
          })

          return stderr.includes('CONFLICT') || stdout.includes('CONFLICT')
        } catch (error2) {
          const err2 = error2 as { stderr?: string; stdout?: string }
          if (err2.stderr?.includes('CONFLICT') || err2.stdout?.includes('CONFLICT')) {
            return true
          }
        }
      }

      return false
    }
  } catch {
    return false
  }
}

interface GitSyncStatusInfo {
  status: GitSyncStatus
  commitsAhead: number
  commitsBehind: number
  hasUncommittedChanges: boolean
  currentBranch: string
  baseBranch: string
  originExists: boolean
}

/**
 * Check sync status with origin/main or origin/master
 */
export async function getOriginSyncStatus(
  repoPath: string,
  baseBranch?: string,
): Promise<GitSyncStatusInfo> {
  try {
    const currentBranch = await getCurrentBranch(repoPath)
    const base = baseBranch || (await detectBaseBranch(repoPath))
    const originExists = await checkAndFetchOrigin(repoPath, base, false)

    if (!originExists) {
      const hasChanges = await hasUncommittedChanges(repoPath)
      return {
        status: hasChanges ? 'uncommitted_changes' : 'synced',
        commitsAhead: 0,
        commitsBehind: 0,
        hasUncommittedChanges: hasChanges,
        currentBranch,
        baseBranch: base,
        originExists: false,
      }
    }

    const hasChanges = await hasUncommittedChanges(repoPath)
    const { ahead, behind } = await getOriginCommitCounts(repoPath, base)

    let status: GitSyncStatus
    if (hasChanges) {
      status = 'uncommitted_changes'
    } else if (ahead > 0 && behind > 0) {
      status = 'diverged'
    } else if (ahead > 0) {
      status = 'ahead'
    } else if (behind > 0) {
      status = 'behind'
    } else {
      status = 'synced'
    }

    return {
      status,
      commitsAhead: ahead,
      commitsBehind: behind,
      hasUncommittedChanges: hasChanges,
      currentBranch,
      baseBranch: base,
      originExists: true,
    }
  } catch (error) {
    logger.error({ err: error as Error }, 'Failed to get origin sync status')
    const hasChanges = await hasUncommittedChanges(repoPath).catch(() => false)
    const currentBranch = await getCurrentBranch(repoPath).catch(() => 'unknown')
    const base = baseBranch || 'main'

    return {
      status: 'error',
      commitsAhead: 0,
      commitsBehind: 0,
      hasUncommittedChanges: hasChanges,
      currentBranch,
      baseBranch: base,
      originExists: false,
    }
  }
}

export async function getOriginCommitCounts(
  repoPath: string,
  baseBranch?: string,
): Promise<{ ahead: number; behind: number }> {
  try {
    const base = baseBranch || (await detectBaseBranch(repoPath))
    const originBranch = `origin/${base}`

    try {
      await execAsync(`git rev-parse --verify ${originBranch}`, {
        cwd: repoPath,
        timeout: 5000,
      })
    } catch {
      return getCommitCounts(repoPath, base)
    }

    const { stdout } = await execAsync(`git rev-list --left-right --count ${originBranch}...HEAD`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const [behind, ahead] = stdout
      .trim()
      .split('\t')
      .map((n) => Number.parseInt(n, 10))

    return { ahead: ahead || 0, behind: behind || 0 }
  } catch (error) {
    logger.warn(
      {
        error: error instanceof Error ? error.message : String(error),
      },
      'Failed to get origin commit counts, falling back to local',
    )
    return getCommitCounts(repoPath, baseBranch)
  }
}

export async function getCommitCounts(
  repoPath: string,
  baseBranch?: string,
): Promise<{ ahead: number; behind: number }> {
  try {
    const base = baseBranch || 'main'

    const { stdout } = await execAsync(`git rev-list --left-right --count ${base}...HEAD`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const [behind, ahead] = stdout
      .trim()
      .split('\t')
      .map((n) => Number.parseInt(n, 10))
    return { ahead: ahead || 0, behind: behind || 0 }
  } catch {
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --left-right --count master...HEAD', {
          cwd: repoPath,
          timeout: 5000,
        })

        const [behind, ahead] = stdout
          .trim()
          .split('\t')
          .map((n) => Number.parseInt(n, 10))
        return { ahead: ahead || 0, behind: behind || 0 }
      } catch {
        return { ahead: 0, behind: 0 }
      }
    }
    return { ahead: 0, behind: 0 }
  }
}

/**
 * Get numstat data to identify files with large changes
 */
async function getNumstatData(
  repoPath: string,
  compareBase: string,
): Promise<Map<string, { additions: number; deletions: number; isBinary: boolean }>> {
  try {
    const { stdout } = await execAsync(`git diff --numstat ${compareBase}`, {
      cwd: repoPath,
      timeout: 10000,
      maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
    })

    const numstatMap = new Map<
      string,
      { additions: number; deletions: number; isBinary: boolean }
    >()

    const lines = stdout
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    for (const line of lines) {
      const parts = line.split('\t')
      if (parts.length < 3) {
        continue
      }

      const addStr = parts[0]
      const delStr = parts[1]
      const filename = parts[2]
      if (!(addStr && delStr && filename)) {
        continue
      }

      const isBinary = addStr === '-' && delStr === '-'

      numstatMap.set(filename, {
        additions: isBinary ? 0 : Number.parseInt(addStr, 10) || 0,
        deletions: isBinary ? 0 : Number.parseInt(delStr, 10) || 0,
        isBinary,
      })
    }

    return numstatMap
  } catch (error) {
    logger.warn(
      { error: error instanceof Error ? error.message : String(error) },
      'Failed to get numstat data, continuing without pre-filtering',
    )
    return new Map()
  }
}

/**
 * Get detailed git diff with file contents and staging information
 */
export async function getGitDiffDetailed(
  repoPath: string,
  baseBranch?: string,
): Promise<GitDiffResponse> {
  try {
    const [diffString, compareBase] = await getGitDiffString(repoPath, baseBranch)
    const { staged, untracked } = await getGitStatus(repoPath)
    const stagedSet = new Set(staged)

    // Get numstat data to identify large files
    const numstatMap = await getNumstatData(repoPath, compareBase)

    // Identify files to skip based on size limits
    const skippedFiles: SkippedFile[] = []
    const filesToSkip = new Set<string>()

    for (const [filename, stats] of numstatMap.entries()) {
      const totalLines = stats.additions + stats.deletions

      if (stats.isBinary) {
        skippedFiles.push({
          filename,
          reason: 'binary',
        })
        filesToSkip.add(filename)
      } else if (totalLines > DIFF_SIZE_LIMITS.MAX_FILE_LINES) {
        skippedFiles.push({
          filename,
          reason: 'too_large',
          additions: stats.additions,
          deletions: stats.deletions,
        })
        filesToSkip.add(filename)
        logger.info(
          { filename, totalLines, limit: DIFF_SIZE_LIMITS.MAX_FILE_LINES },
          'Skipping file due to large diff',
        )
      }
    }

    const diffChunks = diffString.split(/\ndiff --git /).filter((chunk) => chunk.trim())
    const parsedFiles = await parseDiffChunks(
      diffChunks,
      repoPath,
      compareBase,
      stagedSet,
      filesToSkip,
    )
    const untrackedFiles = await addUntrackedFiles(untracked, parsedFiles.files, repoPath)

    const allFiles = [...parsedFiles.files, ...untrackedFiles.files]
    const totalAdditions = parsedFiles.totalAdditions + untrackedFiles.totalAdditions
    const totalDeletions = parsedFiles.totalDeletions

    // Generate warnings for user
    const warnings: string[] = []
    if (skippedFiles.length > 0) {
      const largeCount = skippedFiles.filter((f) => f.reason === 'too_large').length
      const binaryCount = skippedFiles.filter((f) => f.reason === 'binary').length
      if (largeCount > 0) {
        warnings.push(
          `${largeCount} file(s) skipped due to large diffs (>${DIFF_SIZE_LIMITS.MAX_FILE_LINES.toLocaleString()} lines changed)`,
        )
      }
      if (binaryCount > 0) {
        warnings.push(`${binaryCount} binary file(s) excluded from diff`)
      }
    }

    const response: GitDiffResponse = {
      files: allFiles,
      totalChanges: allFiles.length,
      totalAdditions,
      totalDeletions,
      diffString,
      compareBase,
    }

    if (skippedFiles.length > 0) {
      response.skippedFiles = skippedFiles
    }
    if (warnings.length > 0) {
      response.warnings = warnings
    }

    return response
  } catch (error) {
    throw new Error(
      `Failed to get detailed git diff: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

async function parseDiffChunks(
  diffChunks: string[],
  repoPath: string,
  compareBase: string,
  stagedSet: Set<string>,
  filesToSkip: Set<string>,
): Promise<{ files: GitDiffFile[]; totalAdditions: number; totalDeletions: number }> {
  const files: GitDiffFile[] = []
  let totalAdditions = 0
  let totalDeletions = 0

  for (const chunk of diffChunks) {
    const file = await parseDiffChunk(chunk, repoPath, compareBase, stagedSet, filesToSkip)
    if (file) {
      files.push(file)
      totalAdditions += file.additions
      totalDeletions += file.deletions
    }
  }

  return { files, totalAdditions, totalDeletions }
}

async function parseDiffChunk(
  chunk: string,
  repoPath: string,
  compareBase: string,
  stagedSet: Set<string>,
  filesToSkip: Set<string>,
): Promise<GitDiffFile | null> {
  const lines = chunk.split('\n')
  if (lines.length === 0) {
    return null
  }

  const fileNames = extractFileNames(lines[0])
  if (!fileNames) {
    return null
  }

  const { oldFileName, newFileName } = fileNames

  // Skip files that exceed size limits
  if (filesToSkip.has(newFileName)) {
    logger.debug({ filename: newFileName }, 'Skipping file chunk due to size limit')
    return null
  }

  const statusInfo = determineFileStatus(lines)
  const changes = countChanges(lines)
  const content = await getFileContent(repoPath, newFileName, compareBase, statusInfo, changes)

  return {
    status: statusInfo.status,
    filename: newFileName,
    ...(statusInfo.isRenamedFile &&
      oldFileName !== newFileName &&
      oldFileName !== undefined && { oldFilename: oldFileName }),
    additions: changes.additions,
    deletions: changes.deletions,
    ...(content.oldContent !== undefined && { oldContent: content.oldContent }),
    ...(content.newContent !== undefined && { newContent: content.newContent }),
    isStaged: stagedSet.has(newFileName),
  }
}

function extractFileNames(firstLine: string | undefined): {
  oldFileName: string
  newFileName: string
} | null {
  if (!firstLine) {
    return null
  }

  const fileMatch = firstLine.match(/a\/(.+) b\/(.+)/)
  if (!fileMatch) {
    return null
  }

  const oldFileName = fileMatch[1]
  const newFileName = fileMatch[2]
  if (!(oldFileName && newFileName)) {
    return null
  }

  return { oldFileName, newFileName }
}

function determineFileStatus(lines: string[]): {
  status: GitDiffFile['status']
  isNewFile: boolean
  isDeletedFile: boolean
  isRenamedFile: boolean
} {
  const status: GitDiffFile['status'] = 'modified'
  const isNewFile = false
  const isDeletedFile = false
  const isRenamedFile = false

  for (const line of lines.slice(1, 5)) {
    if (line.includes('new file mode')) {
      return { status: 'added', isNewFile: true, isDeletedFile: false, isRenamedFile: false }
    }
    if (line.includes('deleted file mode')) {
      return { status: 'deleted', isNewFile: false, isDeletedFile: true, isRenamedFile: false }
    }
    if (line.includes('rename from') || line.includes('rename to')) {
      return { status: 'renamed', isNewFile: false, isDeletedFile: false, isRenamedFile: true }
    }
  }

  return { status, isNewFile, isDeletedFile, isRenamedFile }
}

function countChanges(lines: string[]): {
  additions: number
  deletions: number
  addedLines: string[]
  removedLines: string[]
} {
  let additions = 0
  let deletions = 0
  const addedLines: string[] = []
  const removedLines: string[] = []

  for (const line of lines) {
    if (line.startsWith('+') && !line.startsWith('+++')) {
      additions++
      addedLines.push(line.substring(1))
    } else if (line.startsWith('-') && !line.startsWith('---')) {
      deletions++
      removedLines.push(line.substring(1))
    }
  }

  return { additions, deletions, addedLines, removedLines }
}

async function getFileContent(
  repoPath: string,
  filename: string,
  compareBase: string,
  statusInfo: { isNewFile: boolean; isDeletedFile: boolean },
  changes: { addedLines: string[]; removedLines: string[] },
): Promise<{ oldContent?: string; newContent?: string }> {
  if (!(statusInfo.isNewFile || statusInfo.isDeletedFile)) {
    return {}
  }

  try {
    if (statusInfo.isNewFile) {
      return { newContent: await getNewFileContent(repoPath, filename, changes.addedLines) }
    }
    if (statusInfo.isDeletedFile) {
      return {
        oldContent: await getDeletedFileContent(
          repoPath,
          filename,
          compareBase,
          changes.removedLines,
        ),
      }
    }
  } catch (error) {
    logger.debug(
      {
        filename,
        error: error instanceof Error ? error.message : String(error),
      },
      'Error getting file content for diff',
    )
  }

  return {}
}

async function getNewFileContent(
  repoPath: string,
  filename: string,
  addedLines: string[],
): Promise<string> {
  const filePath = path.join(repoPath, filename)
  try {
    return await fs.readFile(filePath, 'utf-8')
  } catch {
    return addedLines.join('\n')
  }
}

async function getDeletedFileContent(
  repoPath: string,
  filename: string,
  compareBase: string,
  removedLines: string[],
): Promise<string> {
  try {
    const { stdout } = await execAsync(`git show ${compareBase}:"${filename}"`, {
      cwd: repoPath,
      timeout: 5000,
      maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
    })
    return stdout
  } catch {
    return removedLines.join('\n')
  }
}

async function addUntrackedFiles(
  untracked: string[],
  existingFiles: GitDiffFile[],
  repoPath: string,
): Promise<{ files: GitDiffFile[]; totalAdditions: number }> {
  const files: GitDiffFile[] = []
  let totalAdditions = 0

  for (const untrackedFile of untracked) {
    if (existingFiles.some((f) => f.filename === untrackedFile)) {
      continue
    }

    const file = await createUntrackedFileEntry(repoPath, untrackedFile)
    if (file) {
      files.push(file)
      totalAdditions += file.additions
    }
  }

  return { files, totalAdditions }
}

async function createUntrackedFileEntry(
  repoPath: string,
  untrackedFile: string,
): Promise<GitDiffFile | null> {
  try {
    const filePath = path.join(repoPath, untrackedFile)
    const content = await fs.readFile(filePath, 'utf-8')
    const lineCount = countLines(content)

    return {
      status: 'added',
      filename: untrackedFile,
      additions: lineCount,
      deletions: 0,
      newContent: content,
      isStaged: false,
    }
  } catch (error) {
    logger.debug(
      {
        filename: untrackedFile,
        error: error instanceof Error ? error.message : String(error),
      },
      'Error reading untracked file',
    )
    return null
  }
}

function countLines(content: string): number {
  const splitLines = content.split('\n')
  return content.endsWith('\n') ? splitLines.length - 1 : splitLines.length
}

/**
 * Get file content with change information for a specific file
 */
export async function getFileChangeInfo(
  repoPath: string,
  filePath: string,
  baseBranch?: string,
): Promise<{
  oldContent: string | null
  additions: number
  deletions: number
  isModified: boolean
  status: 'added' | 'modified' | 'deleted' | 'renamed' | 'unchanged'
}> {
  try {
    const { reference: compareBase } = await getBestCompareReference(repoPath, baseBranch)

    const { stdout: numstatOutput } = await execAsync(
      `git diff --numstat "${compareBase}" -- "${filePath}"`,
      {
        cwd: repoPath,
        timeout: 10000,
      },
    )

    if (!numstatOutput.trim()) {
      return {
        oldContent: null,
        additions: 0,
        deletions: 0,
        isModified: false,
        status: 'unchanged',
      }
    }

    const [additionsStr, deletionsStr] = numstatOutput.trim().split('\t')
    const additions = Number.parseInt(additionsStr || '0', 10) || 0
    const deletions = Number.parseInt(deletionsStr || '0', 10) || 0

    // Get file status
    const { stdout: statusOutput } = await execAsync(
      `git diff --name-status "${compareBase}" -- "${filePath}"`,
      {
        cwd: repoPath,
        timeout: 10000,
      },
    )

    let status: 'added' | 'modified' | 'deleted' | 'renamed' = 'modified'
    if (statusOutput.trim()) {
      const statusCode = statusOutput.trim().split('\t')[0]
      if (statusCode?.startsWith('A')) {
        status = 'added'
      } else if (statusCode?.startsWith('D')) {
        status = 'deleted'
      } else if (statusCode?.startsWith('R')) {
        status = 'renamed'
      }
    }

    // Get old content for modified/deleted files
    let oldContent: string | null = null
    if (status === 'modified' || status === 'deleted') {
      try {
        const { stdout } = await execAsync(`git show ${compareBase}:"${filePath}"`, {
          cwd: repoPath,
          timeout: 5000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })
        oldContent = stdout
      } catch {
        // File doesn't exist at that revision
      }
    }

    return {
      oldContent,
      additions,
      deletions,
      isModified: true,
      status,
    }
  } catch (error) {
    throw new Error(
      `Failed to get file change info: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

/**
 * Get working directory changes (staged + unstaged + untracked)
 */
export async function getWorkingDirectoryChanges(repoPath: string): Promise<GitChanges> {
  try {
    const currentBranch = await getCurrentBranch(repoPath)
    const gitStatus = await getGitStatus(repoPath)

    if (gitStatus.clean) {
      return {
        currentBranch,
        status: 'clean',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    const { stdout: diffStatOutput } = await execAsync('git diff --stat HEAD --numstat', {
      cwd: repoPath,
      timeout: 10000,
    })

    let totalAdditions = 0
    let totalDeletions = 0
    const modifiedFiles = new Set<string>()

    const lines = diffStatOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    for (const line of lines) {
      const parts = line.split('\t')
      if (parts.length >= 3) {
        const additions = Number.parseInt(parts[0] || '0', 10) || 0
        const deletions = Number.parseInt(parts[1] || '0', 10) || 0
        const filename = parts[2]

        if (filename && !Number.isNaN(additions) && !Number.isNaN(deletions)) {
          totalAdditions += additions
          totalDeletions += deletions
          modifiedFiles.add(filename)
        }
      }
    }

    // Count untracked files and their lines
    for (const untrackedFile of gitStatus.untracked) {
      try {
        const filePath = path.join(repoPath, untrackedFile)
        const content = await fs.readFile(filePath, 'utf-8')
        const lines = content.split('\n').length
        totalAdditions += lines
        modifiedFiles.add(untrackedFile)
      } catch (error) {
        logger.debug(
          {
            filename: untrackedFile,
            error: error instanceof Error ? error.message : String(error),
          },
          'Error reading untracked file for line count',
        )
      }
    }

    return {
      currentBranch,
      status: 'modified',
      additions: totalAdditions,
      deletions: totalDeletions,
      modifications: modifiedFiles.size,
    }
  } catch (error) {
    logger.error({ err: error as Error, repoPath }, 'Failed to get working directory changes')

    const currentBranch = await getCurrentBranch(repoPath).catch(() => 'unknown')
    return {
      currentBranch,
      status: 'error',
      additions: 0,
      deletions: 0,
      modifications: 0,
    }
  }
}
