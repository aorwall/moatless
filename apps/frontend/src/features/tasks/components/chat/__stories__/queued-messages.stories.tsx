import type { UiMessage } from '@moatless/api-client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { QueuedMessages } from '../queued-messages'

const meta = {
  title: 'Chat/QueuedMessages',
  component: QueuedMessages,
  parameters: {
    layout: 'padded',
  },
  decorators: [
    (Story) => (
      <div className="max-w-3xl mx-auto bg-background border rounded-lg p-0 overflow-hidden">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof QueuedMessages>

export default meta
type Story = StoryObj<typeof meta>

const createQueuedMessage = (content: string, id: string): UiMessage => ({
  uuid: id,
  role: 'user',
  content: [{ type: 'text', text: content }],
  toolResults: [],
  isQueued: true,
})

export const SingleMessage: Story = {
  args: {
    messages: [createQueuedMessage('Fix the login button styling', 'q-1')],
  },
}

export const MultipleMessages: Story = {
  args: {
    messages: [
      createQueuedMessage('Fix the login button styling', 'q-1'),
      createQueuedMessage('Also update the color scheme to match the new design', 'q-2'),
      createQueuedMessage('And add a loading spinner to the submit button', 'q-3'),
    ],
  },
}

export const LongMessageText: Story = {
  args: {
    messages: [
      createQueuedMessage(
        'Please refactor the entire authentication module to use JWT tokens instead of session-based auth, and make sure to update all the middleware functions and route handlers accordingly',
        'q-1',
      ),
      createQueuedMessage('Also run the tests', 'q-2'),
    ],
  },
}

export const Empty: Story = {
  args: {
    messages: [],
  },
}

export const ManyMessages: Story = {
  args: {
    messages: [
      createQueuedMessage('First change request', 'q-1'),
      createQueuedMessage('Second change request', 'q-2'),
      createQueuedMessage('Third change request', 'q-3'),
      createQueuedMessage('Fourth change request', 'q-4'),
      createQueuedMessage('Fifth change request', 'q-5'),
    ],
  },
}
