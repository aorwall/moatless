/**
 * Git utilities for the file explorer server.
 * Adapted from apps/sandbox/src/features/git/utils/git.utils.ts
 * Self-contained — no dependencies on sandbox packages.
 */
import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'

export const execAsync = promisify(exec)

const DIFF_SIZE_LIMITS = {
  MAX_FILE_LINES: 10_000,
  MAX_TOTAL_FILES: 3_000,
  MAX_BUFFER_MB: 50,
} as const

interface SkippedFile {
  filename: string
  reason: 'too_large' | 'binary'
  additions?: number
  deletions?: number
}

export interface GitDiffFile {
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  filename: string
  oldFilename?: string
  additions: number
  deletions: number
  oldContent?: string
  newContent?: string
  isStaged: boolean
}

export interface GitDiffResponse {
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
  const { stdout } = await execAsync('git branch --show-current', {
    cwd: repoPath,
    timeout: 5000,
  })
  return stdout.trim()
}

export async function hasUncommittedChanges(repoPath: string): Promise<boolean> {
  const { stdout } = await execAsync('git status --porcelain', {
    cwd: repoPath,
    timeout: 5000,
  })
  return stdout.trim().length > 0
}

async function getUntrackedFilesDiff(repoPath: string): Promise<string> {
  try {
    const { stdout: untrackedOutput } = await execAsync(
      'git ls-files --others --exclude-standard',
      { cwd: repoPath, timeout: 10000 },
    )

    const untrackedFiles = untrackedOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    let untrackedDiffs = ''
    for (const filename of untrackedFiles) {
      if (!filename) {
        continue
      }
      try {
        const { stdout: fileContent } = await execAsync(`cat "${filename}"`, {
          cwd: repoPath,
          timeout: 5000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })

        untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
        untrackedDiffs += 'new file mode 100644\n'
        untrackedDiffs += 'index 0000000..0000000\n'
        untrackedDiffs += '--- /dev/null\n'
        untrackedDiffs += `+++ b/${filename}\n`

        const lines = fileContent.split('\n')
        if (lines.length > 0) {
          untrackedDiffs += `@@ -0,0 +1,${lines.length} @@\n`
          for (const line of lines) {
            untrackedDiffs += `+${line}\n`
          }
        }
      } catch {
        untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
        untrackedDiffs += 'new file mode 100644\n'
        untrackedDiffs += 'Binary file\n'
      }
    }

    return untrackedDiffs
  } catch {
    return ''
  }
}

export async function detectBaseBranch(repoPath: string): Promise<string> {
  try {
    await execAsync('git rev-parse --verify main', { cwd: repoPath, timeout: 5000 })
    return 'main'
  } catch {
    try {
      await execAsync('git rev-parse --verify master', { cwd: repoPath, timeout: 5000 })
      return 'master'
    } catch {
      return 'main'
    }
  }
}

export async function getMergeBase(
  repoPath: string,
  branch1: string,
  branch2: string,
): Promise<string> {
  const { stdout } = await execAsync(`git merge-base "${branch1}" "${branch2}"`, {
    cwd: repoPath,
    timeout: 5000,
  })
  return stdout.trim()
}

export async function getBestCompareReference(
  repoPath: string,
  baseBranch?: string,
): Promise<{ reference: string; type: 'origin' | 'local' | 'merge-base' }> {
  const base = baseBranch || (await detectBaseBranch(repoPath))
  const currentBranch = await getCurrentBranch(repoPath)
  const originBranch = `origin/${base}`

  try {
    await execAsync(`git rev-parse --verify ${originBranch}`, { cwd: repoPath, timeout: 5000 })
    return { reference: originBranch, type: 'origin' }
  } catch {
    // Origin branch doesn't exist
  }

  try {
    const mergeBase = await getMergeBase(repoPath, base, currentBranch)
    return { reference: mergeBase, type: 'merge-base' }
  } catch {
    return { reference: base, type: 'local' }
  }
}

export async function checkAndFetchOrigin(
  repoPath: string,
  baseBranch?: string,
  shouldFetch = true,
): Promise<boolean> {
  try {
    await execAsync('git remote get-url origin', { cwd: repoPath, timeout: 5000 })
    if (shouldFetch) {
      const base = baseBranch || (await detectBaseBranch(repoPath))
      try {
        await execAsync(`git fetch origin ${base}`, { cwd: repoPath, timeout: 30000 })
      } catch {
        // Fetch failed, non-fatal
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
    await checkAndFetchOrigin(repoPath, baseBranch, false)
    const { reference: compareBase } = await getBestCompareReference(repoPath, baseBranch)

    const { stdout: diffOutput } = await execAsync(`git diff ${compareBase}`, {
      cwd: repoPath,
      timeout: 30000,
      maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
    })

    const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
    const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')
    return [combinedDiff, compareBase]
  } catch (error) {
    if (!baseBranch) {
      try {
        const { reference: fallbackCompare } = await getBestCompareReference(repoPath, 'master')
        const { stdout: diffOutput } = await execAsync(`git diff ${fallbackCompare}`, {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })
        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        return [diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : ''), fallbackCompare]
      } catch {
        const { stdout: diffOutput } = await execAsync('git diff', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })
        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        return [diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : ''), 'HEAD']
      }
    }
    throw new Error(
      `Failed to get git diff: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function isGitRepository(repoPath: string): Promise<boolean> {
  try {
    await execAsync('git rev-parse --git-dir', { cwd: repoPath, timeout: 5000 })
    return true
  } catch {
    return false
  }
}

export async function getGitStatus(repoPath: string): Promise<{
  clean: boolean
  modified: string[]
  untracked: string[]
  staged: string[]
}> {
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

  return { clean: lines.length === 0, modified, untracked, staged }
}

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
      const [addStr, delStr, filename] = parts
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
  } catch {
    return new Map()
  }
}

export async function getGitDiffDetailed(
  repoPath: string,
  baseBranch?: string,
): Promise<GitDiffResponse> {
  const [diffString, compareBase] = await getGitDiffString(repoPath, baseBranch)
  const { staged, untracked } = await getGitStatus(repoPath)
  const stagedSet = new Set(staged)
  const numstatMap = await getNumstatData(repoPath, compareBase)

  const skippedFiles: SkippedFile[] = []
  const filesToSkip = new Set<string>()

  for (const [filename, stats] of numstatMap.entries()) {
    const totalLines = stats.additions + stats.deletions
    if (stats.isBinary) {
      skippedFiles.push({ filename, reason: 'binary' })
      filesToSkip.add(filename)
    } else if (totalLines > DIFF_SIZE_LIMITS.MAX_FILE_LINES) {
      skippedFiles.push({
        filename,
        reason: 'too_large',
        additions: stats.additions,
        deletions: stats.deletions,
      })
      filesToSkip.add(filename)
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
  if (filesToSkip.has(newFileName)) {
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

function extractFileNames(
  firstLine: string | undefined,
): { oldFileName: string; newFileName: string } | null {
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
  return { status: 'modified', isNewFile: false, isDeletedFile: false, isRenamedFile: false }
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
      const filePath = path.join(repoPath, filename)
      try {
        return { newContent: await fs.readFile(filePath, 'utf-8') }
      } catch {
        return { newContent: changes.addedLines.join('\n') }
      }
    }
    if (statusInfo.isDeletedFile) {
      try {
        const { stdout } = await execAsync(`git show ${compareBase}:"${filename}"`, {
          cwd: repoPath,
          timeout: 5000,
          maxBuffer: DIFF_SIZE_LIMITS.MAX_BUFFER_MB * 1024 * 1024,
        })
        return { oldContent: stdout }
      } catch {
        return { oldContent: changes.removedLines.join('\n') }
      }
    }
  } catch {
    // Ignore errors
  }

  return {}
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

    try {
      const filePath = path.join(repoPath, untrackedFile)
      const content = await fs.readFile(filePath, 'utf-8')
      const splitLines = content.split('\n')
      const lineCount = content.endsWith('\n') ? splitLines.length - 1 : splitLines.length

      files.push({
        status: 'added',
        filename: untrackedFile,
        additions: lineCount,
        deletions: 0,
        newContent: content,
        isStaged: false,
      })
      totalAdditions += lineCount
    } catch {
      // Skip files that can't be read
    }
  }

  return { files, totalAdditions }
}

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
  const { reference: compareBase } = await getBestCompareReference(repoPath, baseBranch)

  const { stdout: numstatOutput } = await execAsync(
    `git diff --numstat "${compareBase}" -- "${filePath}"`,
    { cwd: repoPath, timeout: 10000 },
  )

  if (!numstatOutput.trim()) {
    return { oldContent: null, additions: 0, deletions: 0, isModified: false, status: 'unchanged' }
  }

  const [additionsStr, deletionsStr] = numstatOutput.trim().split('\t')
  const additions = Number.parseInt(additionsStr || '0', 10) || 0
  const deletions = Number.parseInt(deletionsStr || '0', 10) || 0

  const { stdout: statusOutput } = await execAsync(
    `git diff --name-status "${compareBase}" -- "${filePath}"`,
    { cwd: repoPath, timeout: 10000 },
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

  return { oldContent, additions, deletions, isModified: true, status }
}
