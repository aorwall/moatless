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
    <div className="pointer-events-none fixed bottom-0 right-0 z-50 flex items-end gap-3 px-4">
      {windows.map((window) => (
        <div key={window.id} className="pointer-events-auto">
          <ComposeWindow window={window} />
        </div>
      ))}
    </div>
  )
}
