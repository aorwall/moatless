import { exec } from 'node:child_process'
import { promisify } from 'node:util'

const execAsync = promisify(exec)

/**
 * Docker container names have the following constraints:
 * - Must be 1-255 characters long
 * - Can only contain lowercase letters, numbers, hyphens, underscores, and periods
 * - Cannot start or end with a period or hyphen
 */

/**
 * Sanitizes a string to be safe for Docker container names
 */
function sanitizeForContainerName(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9._-]/g, '-') // Replace invalid chars with hyphens
    .replace(/^[.-]+|[.-]+$/g, '') // Remove leading/trailing periods or hyphens
    .replace(/[-_]+/g, '-') // Collapse multiple hyphens/underscores
}

/**
 * Generates a Docker container name in the format: moatless-{repositoryName}-{taskIdSuffix}
 * Ensures the name is within Docker's 255 character limit
 */
export function generateContainerName(repositoryName: string, taskId: string): string {
  const prefix = 'moatless'
  const sanitizedRepoName = sanitizeForContainerName(repositoryName)

  // Take first 8 characters of task ID as suffix for uniqueness
  const taskIdSuffix = taskId.substring(0, 8)

  // Calculate available space for repo name
  const staticPartsLength = prefix.length + taskIdSuffix.length + 2 // 2 hyphens
  const maxRepoNameLength = 255 - staticPartsLength

  // Truncate repo name if needed
  const truncatedRepoName =
    sanitizedRepoName.length > maxRepoNameLength
      ? sanitizedRepoName.substring(0, maxRepoNameLength)
      : sanitizedRepoName

  // Remove trailing hyphens after truncation
  const cleanRepoName = truncatedRepoName.replace(/-+$/, '')

  return `${prefix}-${cleanRepoName}-${taskIdSuffix}`
}

/**
 * Checks if a Docker container exists by name
 */
export async function containerExists(containerName: string): Promise<boolean> {
  try {
    await execAsync(`docker inspect ${containerName}`)
    return true
  } catch {
    return false
  }
}

/**
 * Gets the status of a Docker container
 */
export async function getContainerStatus(
  containerName: string,
): Promise<'running' | 'stopped' | 'not-found'> {
  try {
    const { stdout } = await execAsync(
      `docker inspect --format='{{.State.Status}}' ${containerName}`,
    )
    const status = stdout.trim()
    return status === 'running' ? 'running' : 'stopped'
  } catch {
    return 'not-found'
  }
}

/**
 * Gets the container ID for a given container name
 */
export async function getContainerIdByName(containerName: string): Promise<string | null> {
  try {
    const { stdout } = await execAsync(`docker inspect --format='{{.Id}}' ${containerName}`)
    return stdout.trim()
  } catch {
    return null
  }
}

/**
 * Starts an existing stopped container
 */
export async function startExistingContainer(containerName: string): Promise<void> {
  await execAsync(`docker start ${containerName}`)
}

/**
 * Checks if container has the expected labels for a task
 */
export async function validateContainerLabels(
  containerName: string,
  taskId: string,
): Promise<boolean> {
  try {
    const { stdout } = await execAsync(
      `docker inspect --format='{{index .Config.Labels "moatless.task-id"}}' ${containerName}`,
    )
    const containerTaskId = stdout.trim()
    return containerTaskId === taskId
  } catch {
    return false
  }
}
