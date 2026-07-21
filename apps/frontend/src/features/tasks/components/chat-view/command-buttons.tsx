import type { CommandResponse } from '@moatless/api-client'
import { getCommandIcon } from './command-icons'

interface CommandButtonsProps {
  commands: CommandResponse[]
  onCommandClick: (command: CommandResponse) => void
  disabled?: boolean
}

export function CommandButtons({ commands, onCommandClick, disabled }: CommandButtonsProps) {
  if (commands.length === 0) {
    return null
  }

  return (
    <div className="relative w-full rounded-t-lg bg-background py-2">
      <div className="flex w-full select-none flex-nowrap gap-2 overflow-x-auto px-3 pb-1">
        {commands.map((command) => {
          const Icon = getCommandIcon(command.icon ?? undefined)
          return (
            <button
              key={command.name}
              type="button"
              onClick={() => onCommandClick(command)}
              disabled={disabled}
              className="inline-flex h-7 items-center justify-center gap-1.5 whitespace-nowrap rounded-md bg-secondary px-3 py-2 text-sm font-normal text-primary transition-[background-color,opacity,filter] duration-150 hover:brightness-[0.8] active:brightness-[0.65] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Icon className="size-3.5 shrink-0" />
              {command.description}
            </button>
          )
        })}
      </div>
    </div>
  )
}
