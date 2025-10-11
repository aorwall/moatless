import type { Meta, StoryObj } from '@storybook/react-vite'
import { type ComponentProps, useState } from 'react'
import { TaskUIProvider } from '../../../contexts/task-ui-provider'
import { TaskWorkspacePanelView } from '../task-workspace-panel.view'

const meta: Meta<typeof TaskWorkspacePanelView> = {
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

export default meta

type Story = StoryObj<typeof TaskWorkspacePanelView>

export const FilesView: Story = {
  args: {
    activeView: 'files',
    onViewChange: () => undefined,
    isLoading: false,
    error: null,
  },
}

export const PreviewView: Story = {
  args: {
    activeView: 'preview',
    onViewChange: () => undefined,
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    activeView: 'files',
    onViewChange: () => undefined,
    isLoading: true,
    error: null,
  },
}

export const ErrorState: Story = {
  args: {
    activeView: 'files',
    onViewChange: () => undefined,
    isLoading: false,
    error: new Error('Failed to load workspace status'),
  },
}

export const Interactive: Story = {
  render: (args) => <InteractiveWrapper {...args} />,
  args: {
    isLoading: false,
    error: null,
  },
}

function InteractiveWrapper(
  props: Omit<ComponentProps<typeof TaskWorkspacePanelView>, 'activeView' | 'onViewChange'>,
) {
  const [activeView, setActiveView] = useState<'files' | 'preview'>('files')

  return <TaskWorkspacePanelView {...props} activeView={activeView} onViewChange={setActiveView} />
}
