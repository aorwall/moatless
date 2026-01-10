/**
 * Generate a repository-specific email address.
 * Format: {repository}@{inboxDomain}
 *
 * The sender's email address is used to identify the user when
 * processing inbound emails.
 *
 * @param repositoryName - The repository name
 * @param inboxDomain - The inbox domain (e.g., "inbox.moatless.ai")
 * @returns The generated email address
 */
export function generateRepositoryEmailAddress(
  repositoryName: string,
  inboxDomain: string,
): string {
  // Normalize: lowercase, replace spaces with hyphens, remove invalid chars
  const normalizedRepo = repositoryName
    .toLowerCase()
    .replace(/\s+/g, '-')
    .replace(/[^a-z0-9-]/g, '')

  return `${normalizedRepo}@${inboxDomain}`
}
