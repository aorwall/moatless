import { randomBytes } from 'node:crypto'

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz0123456789'
const SHORT_ID_LENGTH = 7 // 36^7 = ~78 billion combinations

/**
 * Generate a URL-safe, lowercase alphanumeric short ID.
 * Used for email subject tags: [TASK-abc1234]
 *
 * @returns A 7-character alphanumeric string
 */
export function generateShortId(): string {
  const bytes = randomBytes(SHORT_ID_LENGTH)
  return Array.from(bytes, (b) => ALPHABET[b % ALPHABET.length]).join('')
}

/**
 * Validate a short ID format.
 * Short IDs must be 6-8 lowercase alphanumeric characters.
 *
 * @param id - The string to validate
 * @returns True if the string is a valid short ID format
 */
export function isValidShortId(id: string): boolean {
  return /^[a-z0-9]{6,8}$/.test(id)
}

/**
 * Extract a task short ID from an email subject line.
 * Looks for patterns like [TASK-abc1234] or [abc1234]
 *
 * @param subject - The email subject line
 * @returns The extracted short ID, or null if not found
 */
export function extractShortIdFromSubject(subject: string): string | null {
  // Try [TASK-abc1234] format first
  const taskMatch = subject.match(/\[TASK-([a-z0-9]{6,8})\]/i)
  if (taskMatch?.[1]) {
    return taskMatch[1].toLowerCase()
  }

  // Try bare [abc1234] format
  const bareMatch = subject.match(/\[([a-z0-9]{6,8})\]/i)
  if (bareMatch?.[1]) {
    return bareMatch[1].toLowerCase()
  }

  return null
}

/**
 * Format a short ID for display in email subjects.
 *
 * @param shortId - The short ID to format
 * @returns Formatted string like [TASK-abc1234]
 */
export function formatShortIdForSubject(shortId: string): string {
  return `[TASK-${shortId}]`
}
