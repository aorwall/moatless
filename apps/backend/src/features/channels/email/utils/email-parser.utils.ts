/**
 * Parse email recipient address to extract repository name.
 * Expected format: {repository}@inbox.moatless.ai
 *
 * @param toAddress - The email recipient address
 * @returns The repository name, or null if invalid format
 */
export function parseRecipientAddress(toAddress: string): string | null {
  // Handle multiple addresses - take the first valid one
  const addresses = toAddress.split(',').map((a) => a.trim())

  for (const address of addresses) {
    // Extract email from "Name <email>" format
    const emailMatch = address.match(/<([^>]+)>/)
    const email = emailMatch?.[1] ?? address

    // Parse {repository}@inbox.moatless.ai format
    const match = email.match(/^([^@]+)@inbox\.moatless\.ai$/i)
    if (match?.[1]) {
      return match[1].toLowerCase()
    }
  }

  return null
}

/**
 * Extract email address from various formats.
 * Handles: "Name <email@domain.com>", "email@domain.com", "<email@domain.com>"
 *
 * @param emailString - The email string to parse
 * @returns The extracted email address
 */
export function extractEmailAddress(emailString: string): string {
  const match = emailString.match(/<([^>]+)>/)
  return match?.[1] ?? emailString.trim()
}

/**
 * Format a task short ID for email subject.
 * Creates [TASK-abc1234] format for threading.
 *
 * @param shortId - The task short ID
 * @param originalSubject - The original email subject
 * @returns Subject with task reference prepended
 */
export function formatSubjectWithTaskId(shortId: string, originalSubject: string): string {
  const prefix = `[TASK-${shortId}]`
  // Avoid duplicate prefixes
  if (originalSubject.includes(prefix)) {
    return originalSubject
  }
  return `${prefix} ${originalSubject}`
}

/**
 * Format reply subject line.
 * Adds "Re: " prefix if not present, includes task reference.
 *
 * @param shortId - The task short ID
 * @param originalSubject - The original email subject
 * @returns Formatted reply subject
 */
export function formatReplySubject(shortId: string, originalSubject: string): string {
  let subject = originalSubject

  // Remove existing Re: prefixes to avoid Re: Re: Re: chains
  subject = subject.replace(/^(re:\s*)+/i, '')

  // Ensure task ID is in subject
  subject = formatSubjectWithTaskId(shortId, subject)

  return `Re: ${subject}`
}

/**
 * Clean up plain text email body for task message.
 * Removes signatures, quoted text, and normalizes whitespace.
 *
 * @param text - The raw email text body
 * @returns Cleaned text suitable for task message
 */
export function cleanEmailBody(text: string): string {
  // Split into lines
  const lines = text.split('\n')
  const cleanedLines: string[] = []

  for (const line of lines) {
    // Stop at common signature markers
    if (line.match(/^--\s*$/)) {
      break
    }
    if (line.match(/^_{10,}$/)) {
      break
    }
    if (line.match(/^-{10,}$/)) {
      break
    }
    if (line.match(/^Sent from my /i)) {
      break
    }
    if (line.match(/^Get Outlook for /i)) {
      break
    }

    // Skip quoted lines (starting with >)
    if (line.trimStart().startsWith('>')) {
      continue
    }

    // Skip common quote headers
    if (line.match(/^On .+ wrote:$/)) {
      break
    }
    if (line.match(/^From: /)) {
      break
    }

    cleanedLines.push(line)
  }

  // Join and normalize whitespace
  return cleanedLines
    .join('\n')
    .trim()
    .replace(/\n{3,}/g, '\n\n')
}
