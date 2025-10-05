import type { Meta, StoryObj } from '@storybook/react'
import { useEffect } from 'react'
import { TaskUIProvider, usePanelView } from '../../contexts/task-ui-provider'
import { WorkspaceHeader } from '../workspace-header'

const meta: Meta<typeof WorkspaceHeader> = {
  title: 'Features/Tasks/WorkspaceHeader',
  component: WorkspaceHeader,
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
}

export default meta

type Story = StoryObj<typeof WorkspaceHeader>

function WorkspaceHeaderStory({ initialView }: { initialView: 'files' | 'preview' }) {
  const { rightPanelView, setRightPanelView } = usePanelView()

  useEffect(() => {
    setRightPanelView(initialView)
  }, [initialView, setRightPanelView])

  return (
    <div className="border-b bg-background">
      <WorkspaceHeader
        centerContent={<span className="text-sm text-muted-foreground">Center slot</span>}
        rightContent={<div className="flex gap-2">Right slot</div>}
      />
      <div className="px-4 pb-2 text-xs text-muted-foreground">
        Active view: <span className="font-mono">{rightPanelView}</span>
      </div>
    </div>
  )
}

export const FilesActive: Story = {
  render: () => <WorkspaceHeaderStory initialView="files" />,
}

export const PreviewActive: Story = {
  render: () => <WorkspaceHeaderStory initialView="preview" />,
}
