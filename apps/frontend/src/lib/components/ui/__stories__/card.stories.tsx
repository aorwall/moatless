import type { Meta, StoryObj } from '@storybook/react-vite'
import { Settings } from 'lucide-react'
import { Button } from '../button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '../card'

const meta = {
  title: 'UI/Card',
  component: Card,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="p-8 bg-background min-w-[400px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Card>

export default meta
type Story = StoryObj<typeof meta>

export const BasicCard: Story = {
  render: () => (
    <Card>
      <CardHeader>
        <CardTitle>Task Analytics</CardTitle>
        <CardDescription>View your task completion statistics</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          <div className="flex justify-between">
            <span className="text-sm text-muted-foreground">Completed</span>
            <span className="font-medium">24</span>
          </div>
          <div className="flex justify-between">
            <span className="text-sm text-muted-foreground">In Progress</span>
            <span className="font-medium">8</span>
          </div>
          <div className="flex justify-between">
            <span className="text-sm text-muted-foreground">Pending</span>
            <span className="font-medium">12</span>
          </div>
        </div>
      </CardContent>
    </Card>
  ),
}

export const WithHeaderAction: Story = {
  render: () => (
    <Card>
      <CardHeader>
        <CardTitle>Repository Settings</CardTitle>
        <CardDescription>Configure your repository preferences</CardDescription>
        <CardAction>
          <Button variant="ghost" size="icon">
            <Settings />
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
          <div>
            <p className="text-sm font-medium">Branch</p>
            <p className="text-sm text-muted-foreground">main</p>
          </div>
          <div>
            <p className="text-sm font-medium">Last commit</p>
            <p className="text-sm text-muted-foreground">2 hours ago</p>
          </div>
          <div>
            <p className="text-sm font-medium">Status</p>
            <p className="text-sm text-muted-foreground">Active</p>
          </div>
        </div>
      </CardContent>
    </Card>
  ),
}

export const WithFooter: Story = {
  render: () => (
    <Card>
      <CardHeader>
        <CardTitle>Create New Task</CardTitle>
        <CardDescription>Start a new coding task or workflow</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
          <div>
            <p className="text-sm font-medium">Task Type</p>
            <p className="text-sm text-muted-foreground">Code generation and modification</p>
          </div>
          <div>
            <p className="text-sm font-medium">Repository</p>
            <p className="text-sm text-muted-foreground">moatless-swarm-ui</p>
          </div>
        </div>
      </CardContent>
      <CardFooter className="justify-end gap-2">
        <Button variant="outline">Cancel</Button>
        <Button>Create Task</Button>
      </CardFooter>
    </Card>
  ),
}
