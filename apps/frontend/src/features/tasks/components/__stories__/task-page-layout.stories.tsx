import type { Meta, StoryObj } from '@storybook/react-vite'
import { TaskPageLayout } from '../task-page-layout'

const _meta: Meta<typeof TaskPageLayout> = {
  title: 'Features/Tasks/TaskPageLayout',
  component: TaskPageLayout,
  parameters: {
    layout: 'fullscreen',
  },
}

type Story = StoryObj<typeof TaskPageLayout>

function HeaderMock() {
  return (
    <div className="border-b bg-background px-4 py-3">
      <div className="flex flex-col gap-1">
        <span className="text-xs uppercase tracking-wide text-muted-foreground">Task</span>
        <div className="flex items-center gap-2">
          <span className="font-semibold">Refactor task page layout</span>
          <span className="text-xs text-muted-foreground">#123</span>
        </div>
      </div>
    </div>
  )
}

function PanelPlaceholder({ label }: { label: string }) {
  return (
    <div className="flex h-full min-h-[240px] flex-1 items-center justify-center bg-muted text-muted-foreground">
      {label}
    </div>
  )
}

export const Default: Story = {
  render: () => (
    <div className="h-[600px] w-full">
      <TaskPageLayout
        header={<HeaderMock />}
        chat={<PanelPlaceholder label="Chat Panel" />}
        workspacePanel={<PanelPlaceholder label="Workspace Panel" />}
      />
    </div>
  ),
}
