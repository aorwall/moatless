/**
 * Utility functions for generating valid Docker container names
 */

/**
 * Generates a valid Docker container name following the pattern:
 * moatless-{repo-name}-{task-id-prefix}
 *
 * Docker container name constraints:
 * - Must be 1-63 characters long
 * - Can only contain lowercase letters, digits, underscores, periods and dashes
 * - Cannot start or end with special characters
 *
 * @param repoName Repository name (may contain special characters)
 * @param taskId Full task ID
 * @returns Valid Docker container name
 */
export function generateContainerName(repoName: string, taskId: string): string {
  // Sanitize repository name for Docker container naming
  const sanitizedRepoName = sanitizeForDockerName(repoName)

  // Take first 8 characters of task ID as prefix
  const taskIdPrefix = taskId.substring(0, 8)

  // Build container name
  const baseName = `moatless-${sanitizedRepoName}-${taskIdPrefix}`

  // Ensure name doesn't exceed Docker's 63 character limit
  if (baseName.length <= 63) {
    return baseName
  }

  // If too long, truncate the repo name part
  const prefixLength = 'moatless-'.length
  const suffixLength = `-${taskIdPrefix}`.length
  const maxRepoNameLength = 63 - prefixLength - suffixLength

  const truncatedRepoName = sanitizedRepoName.substring(0, maxRepoNameLength)
  return `moatless-${truncatedRepoName}-${taskIdPrefix}`
}

/**
 * Sanitizes a string to be valid for Docker container naming
 *
 * @param input Input string
 * @returns Sanitized string valid for Docker names
 */
function sanitizeForDockerName(input: string): string {
  return input
    .toLowerCase() // Convert to lowercase
    .replace(/[^a-z0-9._-]/g, '-') // Replace invalid characters with dashes
    .replace(/^[._-]+|[._-]+$/g, '') // Remove leading/trailing special chars
    .replace(/[._-]{2,}/g, '-') // Replace multiple consecutive special chars with single dash
    .substring(0, 50) // Limit length to leave room for prefix/suffix
}
