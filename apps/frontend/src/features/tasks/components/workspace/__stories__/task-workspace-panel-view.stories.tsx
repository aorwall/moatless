import type { Meta, StoryObj } from '@storybook/react-vite'
import { type ComponentProps, useState } from 'react'
import { TaskUIProvider } from '../../../contexts/task-ui-provider'
import { TaskWorkspacePanelView } from '../task-workspace-panel.view'

const _meta: Meta<typeof TaskWorkspacePanelView> = {
  title: 'Features/Tasks/TaskWorkspacePanelView',
  component: TaskWorkspacePanelView,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <TaskUIProvider taskId="storybook-task">
        <Story />
      </TaskUIProvider>
    ),
  ],
  args: {
    headerCenter: <span className="text-xs text-muted-foreground">Header center slot</span>,
    headerRight: <span className="text-xs text-muted-foreground">Header right slot</span>,
    filesPanel: (
      <div className="flex h-full items-center justify-center bg-muted text-muted-foreground">
        Files panel content
      </div>
    ),
    previewPanel: (
      <div className="flex h-full items-center justify-center bg-muted text-muted-foreground">
        Preview panel content
      </div>
    ),
  },
}

type Story = StoryObj<typeof TaskWorkspacePanelView>

export const Loading: Story = {
  args: {
    activeView: 'files',
    onViewChange: () => undefined,
    isLoading: true,
    error: null,
  },
}

new Error('Failed to load workspace status')

function _InteractiveWrapper(
  props: Omit<ComponentProps<typeof TaskWorkspacePanelView>, 'activeView' | 'onViewChange'>,
) {
  const [activeView, setActiveView] = useState<'files' | 'preview'>('files')

  return <TaskWorkspacePanelView {...props} activeView={activeView} onViewChange={setActiveView} />
}
