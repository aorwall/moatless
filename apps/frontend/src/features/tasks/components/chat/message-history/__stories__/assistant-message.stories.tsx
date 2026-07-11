import type { UiContentBlock, UiMessage } from '@moatless/api-client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { AssistantMessageComponent } from '../assistant-message.tsx'

const meta = {
  title: 'Chat/Messages/Assistant Message',
  component: AssistantMessageComponent,
  parameters: {
    layout: 'padded',
  },
  decorators: [
    (Story) => (
      <div className="max-w-4xl mx-auto p-4 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantMessageComponent>

export default meta
type Story = StoryObj<typeof meta>

// Interactive wrapper component for stateful stories
function InteractiveAssistantMessage({
  message,
  initialExpanded = [],
}: {
  message: UiMessage
  initialExpanded?: string[]
}) {
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set(initialExpanded))

  const handleToggleToolCall = (toolUseId: string) => {
    setExpandedToolCalls((prev) => {
      const newSet = new Set(prev)
      if (newSet.has(toolUseId)) {
        newSet.delete(toolUseId)
      } else {
        newSet.add(toolUseId)
      }
      return newSet
    })
  }

  return (
    <AssistantMessageComponent
      message={message}
      expandedToolCalls={expandedToolCalls}
      onToggleToolCall={handleToggleToolCall}
    />
  )
}

const createAssistantMessage = (content: UiContentBlock[], id = 'assistant-msg-1'): UiMessage => ({
  uuid: id,
  role: 'assistant',
  content,
  toolResults: [],
})

const textContent = (text: string): UiContentBlock[] => [{ type: 'text', text }]

export const SimpleText = {
  render: () => (
    <InteractiveAssistantMessage
      message={createAssistantMessage(
        textContent("I'll help you fix the authentication bug. Let me analyze the code first."),
      )}
    />
  ),
} as unknown as Story

export const WithMarkdown = {
  render: () => (
    <InteractiveAssistantMessage
      message={createAssistantMessage(
        textContent(
          '## Solution Found\n\n' +
            'The issue is in the `authenticate` function:\n\n' +
            '```javascript\n' +
            'function authenticate(user) {\n' +
            '  // Missing null check\n' +
            '  if (!token) {\n' +
            '    throw new Error("Token service not initialized");\n' +
            '  }\n' +
            '  return token.verify(user.token);\n' +
            '}\n' +
            '```\n\n' +
            'This should resolve the **undefined token** error.\n\n' +
            '### Steps to fix:\n\n' +
            '- Initialize the token service before use\n' +
            '- Add proper null checks for the token\n' +
            '- Handle errors gracefully with try-catch\n' +
            '- Update unit tests to cover this scenario',
        ),
      )}
    />
  ),
} as unknown as Story

export const WithMultipleToolCalls = {
  render: () => {
    const content: UiContentBlock[] = [
      {
        type: 'text',
        text: "I'll check both the authentication and token files to understand the issue.",
      },
      {
        type: 'tool_use',
        toolUseId: 'read-auth',
        toolName: 'Read',
        toolInput: {
          file_path: '/src/auth/authenticate.js',
        },
      },
      {
        type: 'tool_use',
        toolUseId: 'read-token',
        toolName: 'Read',
        toolInput: {
          file_path: '/src/auth/token.js',
        },
      },
      {
        type: 'text',
        text: 'Found the issue! The token service needs to be imported.',
      },
    ]
    return (
      <InteractiveAssistantMessage
        message={{
          ...createAssistantMessage(content),
          toolResults: [
            {
              toolUseId: 'read-auth',
              content: 'authenticate.js content here...',
            },
            {
              toolUseId: 'read-token',
              content: 'token.js content here...',
            },
          ],
        }}
      />
    )
  },
} as unknown as Story

export const WithTodoWrite = {
  render: () => {
    const content: UiContentBlock[] = [
      {
        type: 'text',
        text: "I'll create a task list to track the authentication fixes:",
      },
      {
        type: 'tool_use',
        toolUseId: 'todo-1',
        toolName: 'TodoWrite',
        toolInput: {
          todos: [
            {
              id: '1',
              content: 'Analyze authentication flow',
              status: 'completed',
            },
            {
              id: '2',
              content: 'Fix token initialization issue',
              status: 'in_progress',
            },
            {
              id: '3',
              content: 'Add error handling',
              status: 'pending',
            },
            {
              id: '4',
              content: 'Write unit tests',
              status: 'pending',
            },
            {
              id: '5',
              content: 'Update documentation',
              status: 'pending',
            },
          ],
        },
      },
    ]
    return (
      <InteractiveAssistantMessage
        message={{
          ...createAssistantMessage(content),
          toolResults: [
            {
              toolUseId: 'todo-1',
              content: 'Todos have been modified successfully.',
            },
          ],
        }}
      />
    )
  },
} as unknown as Story

export const WithError = {
  render: () => {
    const content: UiContentBlock[] = [
      {
        type: 'text',
        text: 'Let me check that file:',
      },
      {
        type: 'tool_use',
        toolUseId: 'read-error',
        toolName: 'Read',
        toolInput: {
          file_path: '/nonexistent/file.js',
        },
      },
      {
        type: 'text',
        text: "The file doesn't exist. Let me search for it in the project:",
      },
    ]
    return (
      <InteractiveAssistantMessage
        message={{
          ...createAssistantMessage(content),
          toolResults: [
            {
              toolUseId: 'read-error',
              content: 'Error: File not found: /nonexistent/file.js',
            },
          ],
        }}
      />
    )
  },
} as unknown as Story
