import { ValidationError } from '@moatless/types'

/**
 * Parse a GitHub HTTPS URL and extract owner/repo information
 * Only accepts GitHub HTTPS URLs for security
 */
export function parseGitUrl(remoteUrl: string): {
  owner: string
  repoName: string
} {
  // Only accept GitHub HTTPS URLs
  const githubHttpsPattern = /^https:\/\/github\.com\/([^/]+)\/([^/.]+)(?:\.git)?$/

  const match = remoteUrl.match(githubHttpsPattern)
  if (match) {
    return {
      owner: match[1] || '',
      repoName: match[2] || '',
    }
  }

  throw new ValidationError(
    'Invalid GitHub URL format. Only HTTPS URLs are supported (e.g., https://github.com/owner/repo)',
    { url: remoteUrl },
  )
}
