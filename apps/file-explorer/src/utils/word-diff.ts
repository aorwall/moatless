import { diffWords } from 'diff'
import type { WordChange } from '../types/diff'

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
