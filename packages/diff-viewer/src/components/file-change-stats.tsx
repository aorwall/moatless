import type { FileChangeStatsProps } from '../types/index.js'

/**
 * Displays file change statistics (+X / -Y)
 * Styled similar to GitHub's diff stats display
 */
export function FileChangeStats({
  additions,
  deletions,
  showZeros = false,
  size = 'md',
}: FileChangeStatsProps) {
  const showAdditions = additions > 0 || showZeros
  const showDeletions = deletions > 0 || showZeros

  if (!(showAdditions || showDeletions)) {
    return null
  }

  const textSize = size === 'sm' ? 'text-xs' : 'text-sm'

  return (
    <span className={`inline-flex items-center gap-1.5 font-mono ${textSize}`}>
      {showAdditions && <span className="text-green-600 dark:text-green-400">+{additions}</span>}
      {showDeletions && <span className="text-red-600 dark:text-red-400">-{deletions}</span>}
    </span>
  )
}
