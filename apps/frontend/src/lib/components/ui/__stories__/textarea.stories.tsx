import type { Meta, StoryObj } from '@storybook/react'
import { Textarea } from '../textarea'

const meta = {
  title: 'UI/Textarea',
  component: Textarea,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="p-8 bg-background w-[500px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Textarea>

export default meta
type Story = StoryObj<typeof meta>

export const BasicUsage: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">Default Textarea</span>
        <Textarea placeholder="Enter your message here..." />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">With Value</span>
        <Textarea defaultValue="This is a pre-filled textarea with some default content that demonstrates how the component looks with text inside." />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Custom Rows</span>
        <Textarea placeholder="Larger textarea with more rows..." rows={8} />
      </div>
    </div>
  ),
}

export const WithStates: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">Default</span>
        <Textarea placeholder="Enter text..." />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Disabled</span>
        <Textarea placeholder="This textarea is disabled" disabled />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Read Only</span>
        <Textarea defaultValue="This content is read-only and cannot be edited." readOnly />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">With Error (data-form-error)</span>
        <Textarea placeholder="Invalid input" data-form-error="true" />
      </div>
    </div>
  ),
}

export const RealWorldExamples: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <span className="text-sm font-medium">Task Description</span>
        <Textarea placeholder="Describe the task you want to accomplish..." rows={4} />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Code Review Comments</span>
        <Textarea
          placeholder="Add your review comments here..."
          rows={6}
          defaultValue="I reviewed the authentication changes and they look good overall. A few suggestions:

1. Consider adding more error handling for edge cases
2. The token validation logic could be extracted into a separate utility
3. Please add unit tests for the new authenticate() function"
        />
      </div>
      <div className="space-y-2">
        <span className="text-sm font-medium">Commit Message</span>
        <Textarea
          placeholder="Enter commit message..."
          rows={3}
          defaultValue="fix: resolve authentication token initialization issue"
        />
      </div>
    </div>
  ),
}
