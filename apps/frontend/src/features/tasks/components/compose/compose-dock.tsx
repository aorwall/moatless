import { cn } from '@/lib/utils'
import { useComposeTask } from '../../contexts/compose-task-provider'
import { ComposeWindow } from './compose-window'

/**
 * Fixed, bottom-right anchored container that renders every open compose
 * window side-by-side — like Gmail's stacked compose tabs.
 */
export function ComposeDock() {
  const { windows } = useComposeTask()

  if (windows.length === 0) {
    return null
  }

  return (
    <div className="pointer-events-none fixed bottom-0 inset-x-0 z-50 flex justify-end items-end sm:inset-x-auto sm:right-0 sm:gap-3 sm:px-4">
      {windows.map((window) => (
        <div
          key={window.id}
          className={cn('pointer-events-auto sm:w-auto', window.minimized ? 'w-auto' : 'w-full')}
        >
          <ComposeWindow window={window} />
        </div>
      ))}
    </div>
  )
}
