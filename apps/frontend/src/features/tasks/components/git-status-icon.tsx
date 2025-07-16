import type { GitStatus } from '@moatless/types'
import { GitBranch } from 'lucide-react'

interface GitStatusIconProps {
  gitStatus?: GitStatus | null
  className?: string
}

export function GitStatusIcon({ gitStatus, className = '' }: GitStatusIconProps) {
  if (!gitStatus) {
    return null
  }

  // Determine the color based on git status
  const getGitStatusColor = (): string => {
    // Red: conflicts
    if (gitStatus.hasConflicts) {
      return 'text-red-500'
    }

    // Yellow: ahead or uncommitted files
    if (gitStatus.commitsAhead > 0 || gitStatus.hasUncommittedChanges) {
      return 'text-yellow-500'
    }

    // Green: clean state (not ahead, no uncommitted files, no conflicts)
    return 'text-green-500'
  }

  const colorClass = getGitStatusColor()

  return <GitBranch className={`h-3.5 w-3.5 ${colorClass} ${className}`} />
}
