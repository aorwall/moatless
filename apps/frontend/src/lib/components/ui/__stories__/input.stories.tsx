import type { Meta, StoryObj } from '@storybook/react'
import { Input } from '../input'

const meta = {
  title: 'UI/Input',
  component: Input,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="p-8 bg-background w-[400px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Input>

export default meta
type Story = StoryObj<typeof meta>

export const AllTypes: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">Text Input</span>
        <Input type="text" placeholder="Enter your name" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Email Input</span>
        <Input type="email" placeholder="name@example.com" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Password Input</span>
        <Input type="password" placeholder="Enter password" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Number Input</span>
        <Input type="number" placeholder="Enter a number" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">URL Input</span>
        <Input type="url" placeholder="https://example.com" />
      </div>
    </div>
  ),
}

export const WithStates: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">Default</span>
        <Input type="text" placeholder="Enter text" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">With Value</span>
        <Input type="text" defaultValue="moatless-swarm-ui" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Disabled</span>
        <Input type="text" placeholder="Disabled input" disabled />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Read Only</span>
        <Input type="text" defaultValue="Read only value" readOnly />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">With Error (aria-invalid)</span>
        <Input type="text" placeholder="Invalid input" aria-invalid="true" />
      </div>
    </div>
  ),
}

export const WithFile: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">File Upload</span>
        <Input type="file" />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Multiple Files</span>
        <Input type="file" multiple />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Accept Images Only</span>
        <Input type="file" accept="image/*" />
      </div>
    </div>
  ),
}
