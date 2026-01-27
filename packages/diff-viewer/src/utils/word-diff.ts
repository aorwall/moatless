import { diffWords } from 'diff'
import type { WordChange } from '../types/index.js'

/**
 * Compute word-level differences between two strings
 * Returns separate change arrays for old (deleted) and new (added) content
 */
export function computeWordDiff(
  oldContent: string,
  newContent: string,
): { oldChanges: WordChange[]; newChanges: WordChange[] } {
  const changes = diffWords(oldContent, newContent)

  const oldChanges: WordChange[] = []
  const newChanges: WordChange[] = []

  for (const change of changes) {
    if (change.added) {
      // Added content only appears in new
      newChanges.push({
        type: 'add',
        value: change.value,
      })
    } else if (change.removed) {
      // Removed content only appears in old
      oldChanges.push({
        type: 'del',
        value: change.value,
      })
    } else {
      // Unchanged content appears in both
      oldChanges.push({
        type: 'equal',
        value: change.value,
      })
      newChanges.push({
        type: 'equal',
        value: change.value,
      })
    }
  }

  return { oldChanges, newChanges }
}

/**
 * Check if a line has any word-level changes (not all equal)
 */
export function hasWordChanges(changes: WordChange[] | undefined): boolean {
  if (!changes || changes.length === 0) {
    return false
  }
  return changes.some((c) => c.type !== 'equal')
}

/**
 * Tokenize a string into words and whitespace for more granular diffing
 * This preserves whitespace as separate tokens
 */
export function tokenize(text: string): string[] {
  // Split on word boundaries while preserving whitespace
  return text.split(/(\s+)/).filter((token) => token.length > 0)
}

/**
 * Compute character-level differences for very similar strings
 * Useful for small changes like typo fixes
 */
export function computeCharDiff(
  oldContent: string,
  newContent: string,
): { oldChanges: WordChange[]; newChanges: WordChange[] } {
  const oldChanges: WordChange[] = []
  const newChanges: WordChange[] = []

  // Simple character-by-character comparison using longest common subsequence approach
  const oldChars = [...oldContent]
  const newChars = [...newContent]

  let oldIdx = 0
  let newIdx = 0

  while (oldIdx < oldChars.length || newIdx < newChars.length) {
    if (
      oldIdx < oldChars.length &&
      newIdx < newChars.length &&
      oldChars[oldIdx] === newChars[newIdx]
    ) {
      // Characters match
      oldChanges.push({ type: 'equal', value: oldChars[oldIdx] ?? '' })
      newChanges.push({ type: 'equal', value: newChars[newIdx] ?? '' })
      oldIdx++
      newIdx++
    } else if (newIdx >= newChars.length || oldIdx < oldChars.length) {
      // Character deleted
      oldChanges.push({ type: 'del', value: oldChars[oldIdx] ?? '' })
      oldIdx++
    } else {
      // Character added
      newChanges.push({ type: 'add', value: newChars[newIdx] ?? '' })
      newIdx++
    }
  }

  // Merge consecutive changes of the same type
  return {
    oldChanges: mergeChanges(oldChanges),
    newChanges: mergeChanges(newChanges),
  }
}

/**
 * Merge consecutive changes of the same type
 */
function mergeChanges(changes: WordChange[]): WordChange[] {
  if (changes.length === 0) {
    return []
  }

  const merged: WordChange[] = []
  let current = changes[0]

  if (!current) {
    return []
  }

  for (let i = 1; i < changes.length; i++) {
    const change = changes[i]
    if (change && change.type === current.type) {
      current = { type: current.type, value: current.value + change.value }
    } else if (change) {
      merged.push(current)
      current = change
    }
  }
  merged.push(current)

  return merged
}
