import {
  GitBranch,
  GitCommit,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  Play,
  Wrench,
} from 'lucide-react'

export function getCommandIcon(iconName?: string) {
  switch (iconName) {
    case 'GitCommit':
      return GitCommit
    case 'GitBranch':
      return GitBranch
    case 'GitMerge':
      return GitMerge
    case 'GitPullRequest':
      return GitPullRequest
    case 'Play':
      return Play
    case 'Wrench':
      return Wrench
    default:
      return MessageSquare
  }
}
