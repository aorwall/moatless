/**
 * Relaxed JSON parsing utility that extracts JSON objects from text responses
 * even when the response contains additional text or formatting.
 */

export interface JsonParseResult<T> {
  data: T | null
  found: boolean
  error?: string
}

/**
 * Extracts JSON objects from text that may contain additional content.
 * Supports finding JSON objects anywhere in the text and validates required keys.
 *
 * @param text - The text to parse JSON from
 * @param requiredKeys - Array of keys that must be present in the JSON object
 * @returns JsonParseResult with the parsed data or null if not found/valid
 */
export function parseJsonFromText<T = Record<string, unknown>>(
  text: string,
  requiredKeys: string[] = [],
): JsonParseResult<T> {
  if (!text || typeof text !== 'string') {
    return { data: null, found: false, error: 'Invalid input text' }
  }

  // Remove common markdown code block markers
  const cleanedText = text
    .replace(/```json\s*/g, '')
    .replace(/```\s*/g, '')
    .trim()

  // Find potential JSON objects using regex
  const jsonRegex = /\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\}/g
  const matches = cleanedText.match(jsonRegex) || []

  for (const match of matches) {
    try {
      const parsed = JSON.parse(match) as T

      // Validate that it's an object and has required keys
      if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
        const hasRequiredKeys = requiredKeys.every((key) => Object.hasOwn(parsed, key))

        if (hasRequiredKeys) {
          return { data: parsed, found: true }
        }
      }
    } catch {}
  }

  // If no valid JSON found with required keys, try parsing the entire text
  try {
    const parsed = JSON.parse(cleanedText) as T
    if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
      const hasRequiredKeys = requiredKeys.every((key) => Object.hasOwn(parsed, key))

      if (hasRequiredKeys) {
        return { data: parsed, found: true }
      }
    }
  } catch {
    // Ignore parsing errors for full text
  }

  return {
    data: null,
    found: false,
    error:
      requiredKeys.length > 0
        ? `No JSON object found with required keys: ${requiredKeys.join(', ')}`
        : 'No valid JSON object found',
  }
}

/**
 * Specifically parses text to extract branch name and summary information.
 * Looks for JSON objects with 'branchName' and optionally 'summary' keys.
 *
 * @param text - The text response from Claude Code
 * @returns JsonParseResult with branch name and summary data
 */
export function parseBranchNameAndSummary(
  text: string,
): JsonParseResult<{ branchName: string; summary?: string }> {
  return parseJsonFromText<{ branchName: string; summary?: string }>(text, ['branchName'])
}
