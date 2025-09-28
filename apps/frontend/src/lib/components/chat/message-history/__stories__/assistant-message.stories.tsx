import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react'
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
  message: AssistantMessage
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

const createAssistantMessage = (
  content: string | AssistantMessageContent[],
  id = 'assistant-msg-1',
): AssistantMessage => ({
  id,
  role: 'assistant',
  content,
  timestamp: new Date(),
})

export const SimpleText: Story = {
  render: () => (
    <InteractiveAssistantMessage
      message={createAssistantMessage(
        "I'll help you fix the authentication bug. Let me analyze the code first.",
      )}
    />
  ),
}

export const WithMarkdown: Story = {
  render: () => (
    <InteractiveAssistantMessage
      message={createAssistantMessage(
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
          'This should resolve the **undefined token** error.',
      )}
    />
  ),
}

export const WithSingleToolCall: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: 'Let me read the authentication file to understand the issue.',
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-1',
        tool_name: 'Read',
        tool_input: {
          file_path: '/src/auth/authenticate.js',
          limit: 100,
        },
        tool_result: {
          content:
            'function authenticate(user) {\n' +
            '  return token.verify(user.token);\n' +
            '}\n\n' +
            'function logout(user) {\n' +
            '  token.revoke(user.token);\n' +
            '}',
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithMultipleToolCalls: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: "I'll check both the authentication and token files to understand the issue.",
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-auth',
        tool_name: 'Read',
        tool_input: {
          file_path: '/src/auth/authenticate.js',
        },
        tool_result: {
          content: 'authenticate.js content here...',
        },
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-token',
        tool_name: 'Read',
        tool_input: {
          file_path: '/src/auth/token.js',
        },
        tool_result: {
          content: 'token.js content here...',
        },
      },
      {
        type: 'text',
        text: 'Found the issue! The token service needs to be imported.',
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithLongToolResult: Story = {
  render: () => {
    const longResult = Array.from(
      { length: 20 },
      (_, i) => `Line ${i + 1}: This is a sample line of code output from the tool`,
    ).join('\n')

    const content: AssistantMessageContent[] = [
      {
        type: 'tool_use',
        tool_use_id: 'bash-1',
        tool_name: 'Bash',
        tool_input: {
          command: 'npm run test',
        },
        tool_result: {
          content: longResult,
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithTodoWrite: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: "I'll create a task list to track the authentication fixes:",
      },
      {
        type: 'tool_use',
        tool_use_id: 'todo-1',
        tool_name: 'TodoWrite',
        tool_input: {
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
        tool_result: {
          content: 'Todos have been modified successfully.',
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithEdit: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: "I'll fix the authentication bug by adding proper error handling:",
      },
      {
        type: 'tool_use',
        tool_use_id: 'edit-1',
        tool_name: 'Edit',
        tool_input: {
          file_path: '/src/auth/authenticate.js',
          old_string: 'function authenticate(user) {\n  return token.verify(user.token);\n}',
          new_string:
            'function authenticate(user) {\n  if (!token) {\n    throw new Error("Token service not initialized");\n  }\n  return token.verify(user.token);\n}',
        },
        tool_result: {
          content: 'File updated successfully.',
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const OnlyToolCalls: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'tool_use',
        tool_use_id: 'grep-1',
        tool_name: 'Grep',
        tool_input: {
          pattern: 'authenticate',
          path: '/src',
        },
        tool_result: {
          content: 'Found 5 matches in 3 files',
        },
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-1',
        tool_name: 'Read',
        tool_input: {
          file_path: '/src/auth/authenticate.js',
        },
        tool_result: {
          content: 'File content here...',
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithError: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: 'Let me check that file:',
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-error',
        tool_name: 'Read',
        tool_input: {
          file_path: '/nonexistent/file.js',
        },
        tool_result: {
          content: 'Error: File not found: /nonexistent/file.js',
        },
      },
      {
        type: 'text',
        text: "The file doesn't exist. Let me search for it in the project:",
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const ComplexMessage: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text:
          '# Authentication Bug Fix\n\n' +
          "I've identified the issue with the authentication system. Let me fix it step by step:",
      },
      {
        type: 'tool_use',
        tool_use_id: 'todo-complex',
        tool_name: 'TodoWrite',
        tool_input: {
          todos: [
            { id: '1', content: 'Identify the bug', status: 'completed' },
            { id: '2', content: 'Fix token initialization', status: 'in_progress' },
          ],
        },
      },
      {
        type: 'tool_use',
        tool_use_id: 'read-complex',
        tool_name: 'Read',
        tool_input: { file_path: '/src/auth/token.js' },
        tool_result: {
          content: 'export const token = new TokenService();',
        },
      },
      {
        type: 'text',
        text:
          '## Solution\n\n' +
          'The issue was that the `token` variable was not being imported. ' +
          "Here's the fix:\n\n" +
          '```javascript\n' +
          "import { token } from './token';\n\n" +
          'function authenticate(user) {\n' +
          '  if (!token) {\n' +
          '    throw new Error("Token service not initialized");\n' +
          '  }\n' +
          '  return token.verify(user.token);\n' +
          '}\n' +
          '```',
      },
      {
        type: 'tool_use',
        tool_use_id: 'edit-complex',
        tool_name: 'Edit',
        tool_input: {
          file_path: '/src/auth/authenticate.js',
          old_string: 'old code',
          new_string: 'new code',
        },
        tool_result: {
          content: 'Successfully updated the file',
        },
      },
      {
        type: 'text',
        text: '✅ **Fixed!** The authentication should now work correctly.',
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}

export const WithWebFetch: Story = {
  render: () => {
    const content: AssistantMessageContent[] = [
      {
        type: 'text',
        text: 'Let me check the latest documentation for this API:',
      },
      {
        type: 'tool_use',
        tool_use_id: 'web-1',
        tool_name: 'WebFetch',
        tool_input: {
          url: 'https://api.example.com/docs',
          prompt: 'Get authentication requirements',
        },
        tool_result: {
          content: 'API requires OAuth 2.0 authentication with JWT tokens...',
        },
      },
    ]
    return <InteractiveAssistantMessage message={createAssistantMessage(content)} />
  },
}
