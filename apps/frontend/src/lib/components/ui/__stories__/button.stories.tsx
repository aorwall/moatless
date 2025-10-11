import type { Meta, StoryObj } from '@storybook/react-vite'
import { Plus, Trash2, Upload } from 'lucide-react'
import { Button } from '../button'

const meta = {
  title: 'UI/Button',
  component: Button,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="p-8 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Button>

export default meta
type Story = StoryObj<typeof meta>

export const AllVariants: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="flex gap-4 items-center">
        <Button variant="default">Default</Button>
        <Button variant="destructive">Destructive</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="link">Link</Button>
      </div>
      <div className="flex gap-4 items-center">
        <Button variant="default" disabled>
          Disabled
        </Button>
        <Button variant="destructive" disabled>
          Disabled
        </Button>
        <Button variant="outline" disabled>
          Disabled
        </Button>
      </div>
    </div>
  ),
}

export const AllSizes: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="flex gap-4 items-center">
        <Button size="sm">Small</Button>
        <Button size="default">Default</Button>
        <Button size="lg">Large</Button>
      </div>
      <div className="flex gap-4 items-center">
        <Button size="icon" variant="outline">
          <Plus />
        </Button>
        <Button size="icon" variant="default">
          <Upload />
        </Button>
        <Button size="icon" variant="destructive">
          <Trash2 />
        </Button>
      </div>
    </div>
  ),
}

export const WithIcons: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="flex gap-4 items-center">
        <Button>
          <Plus />
          Add Item
        </Button>
        <Button variant="outline">
          <Upload />
          Upload File
        </Button>
        <Button variant="destructive">
          <Trash2 />
          Delete
        </Button>
      </div>
      <div className="flex gap-4 items-center">
        <Button variant="secondary">
          Save Changes
          <Plus />
        </Button>
        <Button variant="ghost">
          Cancel
          <Trash2 />
        </Button>
      </div>
    </div>
  ),
}
