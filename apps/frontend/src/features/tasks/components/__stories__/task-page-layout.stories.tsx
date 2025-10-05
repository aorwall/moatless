import type { Meta, StoryObj } from '@storybook/react'
import { TaskPageLayout } from '../task-page-layout'

const meta: Meta<typeof TaskPageLayout> = {
  title: 'Features/Tasks/TaskPageLayout',
  component: TaskPageLayout,
  parameters: {
    layout: 'fullscreen',
  },
}

export default meta

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

export const ScrollableChat: Story = {
  render: () => (
    <div className="h-[600px] w-full">
      <TaskPageLayout
        header={<HeaderMock />}
        chat={
          <div className="flex h-full flex-col overflow-hidden">
            <div className="flex-1 space-y-2 overflow-auto bg-muted/40 p-4 text-sm">
              {Array.from({ length: 20 }).map((_, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: Static mock data in Storybook
                <p key={index} className="rounded-md bg-background p-2 shadow-sm">
                  Message {index + 1}: Lorem ipsum dolor sit amet, consectetur adipiscing elit.
                </p>
              ))}
            </div>
            <div className="border-t bg-background p-3">
              <div className="h-10 rounded-md border border-dashed border-muted-foreground/40" />
            </div>
          </div>
        }
        workspacePanel={<PanelPlaceholder label="Workspace Panel" />}
      />
    </div>
  ),
}
