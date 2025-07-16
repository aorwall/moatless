import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { getDataDir } from '../../utils/data-dir.js'

/**
 * Sets up a temporary data directory for tests
 * @param prefix - Prefix for the temporary directory name
 * @returns The path to the temporary directory
 */
export async function setupTestDataDir(prefix = 'workspace-test'): Promise<string> {
  const tempDir = await fs.mkdtemp(path.join(tmpdir(), `moatless-${prefix}-`))
  process.env.DATA_DIR = tempDir

  // Create subdirectories that the app expects
  await fs.mkdir(path.join(tempDir, 'tasks'), { recursive: true })
  await fs.mkdir(path.join(tempDir, 'events'), { recursive: true })
  await fs.mkdir(path.join(tempDir, 'repositories'), { recursive: true })
  await fs.mkdir(path.join(tempDir, 'workspaces'), { recursive: true })

  return tempDir
}

/**
 * Cleans up a test data directory
 * @param dataDir - The directory to clean up (optional, uses getDataDir() if not provided)
 */
export async function cleanupTestDataDir(dataDir?: string): Promise<void> {
  const dir = dataDir || getDataDir()
  if (dir && dir.includes(tmpdir())) {
    try {
      await fs.rm(dir, { recursive: true, force: true })
    } catch (error) {
      // Ignore errors during cleanup
      console.warn('Failed to cleanup test data dir:', error)
    }
  }
}

/**
 * Creates a test task file in the data directory
 * @param taskId - The task ID
 * @param taskData - The task data to write
 */
export async function createTestTask(taskId: string, taskData: any): Promise<void> {
  const dataDir = getDataDir()
  const taskPath = path.join(dataDir, 'tasks', `${taskId}.json`)
  await fs.writeFile(taskPath, JSON.stringify(taskData, null, 2))
}

/**
 * Creates a test repository config in the data directory
 * @param repoId - The repository ID
 * @param repoData - The repository data to write
 */
export async function createTestRepository(repoId: string, repoData: any): Promise<void> {
  const dataDir = getDataDir()
  const repoPath = path.join(dataDir, 'repositories', `${repoId}.json`)
  await fs.writeFile(repoPath, JSON.stringify(repoData, null, 2))
}
