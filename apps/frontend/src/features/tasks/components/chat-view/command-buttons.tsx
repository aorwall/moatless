import type { Command } from '@moatless/shared/types/command'
import { GitBranch, GitCommit, GitMerge, GitPullRequest, MessageSquare, Play } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

interface CommandButtonsProps {
  commands: Command[]
  onCommandClick: (command: Command) => void
  disabled?: boolean
}

export function CommandButtons({ commands, onCommandClick, disabled }: CommandButtonsProps) {
  if (commands.length === 0) {
    return null
  }

  const getCommandIcon = (iconName?: string) => {
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
      default:
        return MessageSquare
    }
  }

  return (
    <div className="flex justify-center py-4">
      <div className="flex flex-wrap gap-2 justify-center">
        {commands.map((command) => {
          const Icon = getCommandIcon(command.icon)
          return (
            <Button
              key={command.name}
              variant={command.variant || 'outline'}
              size="sm"
              onClick={() => onCommandClick(command)}
              disabled={disabled}
              className="flex items-center gap-2"
            >
              <Icon className="h-4 w-4" />
              {command.description}
            </Button>
          )
        })}
      </div>
    </div>
  )
}
