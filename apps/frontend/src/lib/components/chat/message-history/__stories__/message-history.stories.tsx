import type { UiContentBlock, UiMessage, UiToolResult } from '@moatless/api-client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import React from 'react'
import { MessageHistory } from '../message-history.tsx'

const meta = {
  title: 'Chat/Messages/Message History',
  component: MessageHistory,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <div className="h-screen bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageHistory>

export default meta
type Story = StoryObj<typeof meta>

// Helper functions to create messages
const createUserMessage = (
  content: string,
  id: string,
  options?: {
    messageType?: string
  },
): UiMessage => ({
  uuid: id,
  role: 'user',
  content: [{ type: 'text', text: content }],
  toolResults: [],
  ...options,
})

const createAssistantMessage = (
  content: UiContentBlock[],
  id: string,
  options?: {
    messageType?: string
    toolResults?: UiToolResult[]
  },
): UiMessage => ({
  uuid: id,
  role: 'assistant',
  content,
  toolResults: options?.toolResults ?? [],
  ...options,
})

const textBlocks = (text: string): UiContentBlock[] => [{ type: 'text', text }]

// Stories
export const EmptyConversation: Story = {
  args: {
    messages: [],
    isLoading: false,
  },
}

export const LoadingState: Story = {
  args: {
    messages: undefined,
    isLoading: true,
  },
}

export const ErrorState: Story = {
  args: {
    messages: undefined,
    isLoading: false,
    error: new Error('Failed to load messages from the server'),
  },
}

export const SimpleConversation: Story = {
  args: {
    messages: [
      createUserMessage('Hello, can you help me with authentication?', 'msg-1'),
      createAssistantMessage(
        textBlocks(
          "Of course! I'd be happy to help you with authentication. What specific aspect are you working on?",
        ),
        'msg-2',
      ),
      createUserMessage('I need to implement JWT token refresh', 'msg-3'),
      createAssistantMessage(
        textBlocks(
          "I'll help you implement JWT token refresh. Let me first check your current authentication setup.",
        ),
        'msg-4',
      ),
    ],
    isLoading: false,
  },
}

export const WithToolCalls: Story = {
  args: {
    messages: [
      createUserMessage('Can you check the authentication file?', 'msg-1'),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "Sure, I'll examine the authentication file for you.",
          },
          {
            type: 'tool_use',
            toolUseId: 'tool-1',
            toolName: 'Read',
            toolInput: {
              file_path: '/src/auth/authenticate.js',
            },
          },
          {
            type: 'text',
            text: 'I found the authentication function. It appears to be missing error handling.',
          },
        ],
        'msg-2',
        {
          toolResults: [
            {
              toolUseId: 'tool-1',
              content: 'function authenticate(user) {\n  return token.verify(user.token);\n}',
            },
          ],
        },
      ),
    ],
    isLoading: false,
  },
}

export const GroupedToolCalls: Story = {
  args: {
    messages: [
      createUserMessage('Search for all authentication files', 'msg-1'),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'grep-1',
            toolName: 'Grep',
            toolInput: { pattern: 'authenticate', path: '/src' },
          },
        ],
        'msg-2',
        {
          toolResults: [{ toolUseId: 'grep-1', content: 'Found 5 matches' }],
        },
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'read-1',
            toolName: 'Read',
            toolInput: { file_path: '/src/auth.js' },
          },
        ],
        'msg-3',
        {
          toolResults: [{ toolUseId: 'read-1', content: 'auth.js content' }],
        },
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'read-2',
            toolName: 'Read',
            toolInput: { file_path: '/src/token.js' },
          },
        ],
        'msg-4',
        {
          toolResults: [{ toolUseId: 'read-2', content: 'token.js content' }],
        },
      ),
      createAssistantMessage(
        textBlocks('Found all the authentication files. They need some refactoring.'),
        'msg-5',
      ),
    ],
    isLoading: false,
  },
}

export const LongConversation: Story = {
  args: {
    messages: [
      createUserMessage('Hello, I need help with authentication', 'msg-1'),
      createAssistantMessage(
        textBlocks('Hi! I can help you with authentication. What issue are you facing?'),
        'msg-2',
      ),
      createUserMessage(
        'I need to review the authentication file at `/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts`',
        'msg-3',
      ),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "I'll read that file for you.",
          },
          {
            type: 'tool_use',
            toolUseId: 'read-1',
            toolName: 'Read',
            toolInput: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts',
            },
          },
        ],
        'msg-4',
        {
          toolResults: [
            {
              toolUseId: 'read-1',
              content:
                'export class TokenValidationService {\n  validate(token: string) { /* ... */ }\n}',
            },
          ],
        },
      ),
      createUserMessage('Can you also check the refresh token service?', 'msg-5'),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'read-2',
            toolName: 'Read',
            toolInput: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/refresh-token/implementations/RefreshTokenService.ts',
            },
          },
        ],
        'msg-6',
        {
          toolResults: [
            {
              toolUseId: 'read-2',
              content:
                'export class RefreshTokenService {\n  refresh(token: string) { /* ... */ }\n}',
            },
          ],
        },
      ),
      createUserMessage('What did you find?', 'msg-7'),
      createAssistantMessage(
        textBlocks(
          'Both services look good. The token validation is working correctly, and the refresh mechanism is properly implemented.',
        ),
        'msg-8',
      ),
      createUserMessage(
        'Here is my config file:\n\n```json\n{\n  "tokenValidator": "/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts",\n  "refreshService": "/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/refresh-token/implementations/RefreshTokenService.ts"\n}\n```',
        'msg-9',
      ),
      createAssistantMessage(
        textBlocks('I see the issue - those paths should be relative. Let me fix that for you.'),
        'msg-10',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'edit-1',
            toolName: 'Edit',
            toolInput: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/configuration/services.config.json',
              old_string:
                '"/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts"',
              new_string:
                '"./services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts"',
            },
          },
        ],
        'msg-11',
        {
          toolResults: [
            {
              toolUseId: 'edit-1',
              content: 'File updated successfully',
            },
          ],
        },
      ),
      createUserMessage('Great! Can you also update the middleware path?', 'msg-12'),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "I'll update the middleware configuration as well.",
          },
          {
            type: 'tool_use',
            toolUseId: 'edit-2',
            toolName: 'Edit',
            toolInput: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/middleware/validation/token-expiry-check/ExpiryValidationMiddleware.ts',
              old_string: 'old code',
              new_string: 'new code',
            },
          },
        ],
        'msg-13',
        {
          toolResults: [
            {
              toolUseId: 'edit-2',
              content:
                'Successfully updated /Users/developer/projects/enterprise-app/backend/src/modules/authentication/middleware/validation/token-expiry-check/ExpiryValidationMiddleware.ts',
            },
          ],
        },
      ),
      createUserMessage('Thanks! Does this affect the hashing utility?', 'msg-14'),
      createAssistantMessage(
        textBlocks(
          "No, the hashing utility at `/Users/developer/projects/enterprise-app/backend/src/modules/authentication/utilities/crypto/hashing/algorithms/SHA256HashingUtility.ts` is independent and doesn't need changes.",
        ),
        'msg-15',
      ),
      createUserMessage('Perfect! One more thing - can you run the tests?', 'msg-16'),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'bash-1',
            toolName: 'Bash',
            toolInput: {
              command: 'npm test',
            },
          },
        ],
        'msg-17',
        {
          toolResults: [
            {
              toolUseId: 'bash-1',
              content: 'Running tests...\n✓ All tests passed\n\nTest files: 25\nTests passed: 150',
            },
          ],
        },
      ),
      createUserMessage('Excellent! Everything is working now.', 'msg-18'),
      createAssistantMessage(
        textBlocks(
          'Great! All the authentication files have been updated and tests are passing. The configuration now uses relative paths which is much better for portability.',
        ),
        'msg-19',
      ),
    ],
    isLoading: false,
  },
}

export const WithTodos: Story = {
  args: {
    messages: [
      createUserMessage('Help me fix the authentication bugs', 'msg-1'),
      createAssistantMessage(
        [
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
                  content: 'Fix token initialization',
                  status: 'completed',
                },
                {
                  id: '3',
                  content: 'Add error handling',
                  status: 'in_progress',
                },
                { id: '4', content: 'Write unit tests', status: 'pending' },
                { id: '5', content: 'Update documentation', status: 'pending' },
              ],
            },
          },
        ],
        'msg-2',
        {
          toolResults: [{ toolUseId: 'todo-1', content: 'Todos updated' }],
        },
      ),
      createUserMessage('Great! How is the progress?', 'msg-3'),
      createAssistantMessage(
        textBlocks(
          "I've completed the analysis and fixed the token initialization. Currently working on adding error handling.",
        ),
        'msg-4',
      ),
    ],
    isLoading: false,
  },
}

export const MixedContent: Story = {
  args: {
    messages: [
      createUserMessage(
        '## Project Setup\n\n' +
          'I need help setting up authentication for my app with these requirements:\n' +
          '- JWT tokens\n' +
          '- Refresh tokens\n' +
          '- Session management',
        'msg-1',
      ),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "I'll help you set up a complete authentication system. Let me start by checking your current setup:",
          },
          {
            type: 'tool_use',
            toolUseId: 'glob-1',
            toolName: 'Glob',
            toolInput: { pattern: '**/auth*.{js,ts}' },
          },
          {
            type: 'text',
            text: "I see you don't have any authentication files yet. Let me create the structure:",
          },
        ],
        'msg-2',
        {
          toolResults: [{ toolUseId: 'glob-1', content: 'No authentication files found' }],
        },
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'todo-2',
            toolName: 'TodoWrite',
            toolInput: {
              todos: [
                { id: '1', content: 'Create auth service', status: 'pending' },
                {
                  id: '2',
                  content: 'Implement JWT handling',
                  status: 'pending',
                },
                {
                  id: '3',
                  content: 'Add refresh token logic',
                  status: 'pending',
                },
              ],
            },
          },
        ],
        'msg-3',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'write-1',
            toolName: 'Write',
            toolInput: {
              file_path: '/src/auth/authService.js',
              content: 'class AuthService { ... }',
            },
          },
        ],
        'msg-4',
        {
          toolResults: [{ toolUseId: 'write-1', content: 'File created successfully' }],
        },
      ),
      createUserMessage('Can you add error handling?', 'msg-5'),
      createAssistantMessage(
        textBlocks(
          '```javascript\n' +
            'try {\n' +
            '  const token = await authService.authenticate(credentials);\n' +
            '  return { success: true, token };\n' +
            '} catch (error) {\n' +
            '  console.error("Authentication failed:", error);\n' +
            '  return { success: false, error: error.message };\n' +
            '}\n' +
            '```',
        ),
        'msg-6',
      ),
    ],
    isLoading: false,
  },
}

export const ThinkingState: Story = {
  args: {
    messages: [
      createUserMessage('Can you help me refactor the authentication module?', 'msg-1'),
      createAssistantMessage(
        textBlocks(
          "I'll help you refactor the authentication module. Let me first analyze the current code structure.",
        ),
        'msg-2',
      ),
      createUserMessage('Also check if there are any security issues', 'msg-3'),
    ],
    isLoading: false,
    isRunning: true,
  },
}

export const AutoScrollBehavior: Story = {
  render: () => {
    const [messages, setMessages] = React.useState<UiMessage[]>([
      createUserMessage('Hello! Can you help me with auto-scrolling?', 'msg-1'),
      createAssistantMessage(
        textBlocks('Sure! Let me demonstrate how auto-scrolling works when new messages arrive.'),
        'msg-2',
      ),
    ])
    const [isAdding, setIsAdding] = React.useState(false)
    const counterRef = React.useRef(3)

    const addMessage = React.useCallback(() => {
      const newId = `msg-${counterRef.current}`
      const isUser = counterRef.current % 2 === 1
      const newMessage = isUser
        ? createUserMessage(`User message #${Math.floor(counterRef.current / 2) + 1}`, newId)
        : createAssistantMessage(
            textBlocks(`Assistant response #${Math.floor(counterRef.current / 2)}`),
            newId,
          )

      setMessages((prev) => [...prev, newMessage])
      counterRef.current += 1
    }, [])

    const startAutoAdd = () => {
      setIsAdding(true)
    }

    const stopAutoAdd = () => {
      setIsAdding(false)
    }

    React.useEffect(() => {
      if (!isAdding) {
        return
      }

      const interval = setInterval(() => {
        addMessage()
      }, 2000)

      return () => clearInterval(interval)
    }, [isAdding, addMessage])

    return (
      <div className="h-screen flex flex-col bg-background">
        <div className="border-b border-border p-4 bg-muted/30">
          <h2 className="text-lg font-semibold mb-2">Auto-Scroll Behavior Test</h2>
          <p className="text-sm text-muted-foreground mb-4">
            Try scrolling up in the message history, then add a new message. Auto-scroll should only
            work if you're at the bottom.
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={addMessage}
              className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
            >
              Add Message
            </button>
            {!isAdding ? (
              <button
                type="button"
                onClick={startAutoAdd}
                className="px-4 py-2 bg-secondary text-secondary-foreground rounded-md hover:bg-secondary/80 text-sm"
              >
                Start Auto-Add (2s interval)
              </button>
            ) : (
              <button
                type="button"
                onClick={stopAutoAdd}
                className="px-4 py-2 bg-destructive text-destructive-foreground rounded-md hover:bg-destructive/90 text-sm"
              >
                Stop Auto-Add
              </button>
            )}
            <button
              type="button"
              onClick={() => {
                setMessages([
                  createUserMessage('Hello! Can you help me with auto-scrolling?', 'msg-1'),
                  createAssistantMessage(
                    textBlocks(
                      'Sure! Let me demonstrate how auto-scrolling works when new messages arrive.',
                    ),
                    'msg-2',
                  ),
                ])
                counterRef.current = 3
                setIsAdding(false)
              }}
              className="px-4 py-2 bg-muted text-foreground rounded-md hover:bg-muted/80 text-sm border border-border"
            >
              Reset
            </button>
          </div>
        </div>
        <MessageHistory messages={messages} />
      </div>
    )
  },
  parameters: {
    layout: 'fullscreen',
  },
}

export const WithHiddenMessages: Story = {
  args: {
    messages: [
      createUserMessage('Help me understand the authentication flow', 'msg-1'),
      createAssistantMessage(
        textBlocks("I'll analyze the authentication system for you."),
        'msg-2',
      ),
      createUserMessage(
        'This is context that was provided to the agent about the project setup and configuration.',
        'msg-3',
        { messageType: 'transcript_only' },
      ),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "I've reviewed the authentication flow. Here's what I found:",
          },
          {
            type: 'tool_use',
            toolUseId: 'read-1',
            toolName: 'Read',
            toolInput: { file_path: '/src/auth/service.ts' },
          },
        ],
        'msg-4',
        {
          toolResults: [{ toolUseId: 'read-1', content: 'Authentication service code...' }],
        },
      ),
      createAssistantMessage(
        textBlocks('Internal reasoning about the code structure and potential issues.'),
        'msg-5',
        { messageType: 'transcript_only' },
      ),
      createAssistantMessage(
        textBlocks(
          'The authentication flow looks good, but I recommend adding error handling for token expiry.',
        ),
        'msg-6',
      ),
    ],
    isLoading: false,
  },
}

export const WithSummaryMessages: Story = {
  args: {
    messages: [
      createUserMessage(
        '## Previous Conversation Summary\n\n' +
          'Topics discussed:\n' +
          '- Project architecture\n' +
          '- Database schema design\n' +
          '- API endpoint structure\n\n' +
          'Decisions made:\n' +
          '1. Use PostgreSQL\n' +
          '2. REST API with versioning\n' +
          '3. JWT authentication',
        'summary-1',
        { messageType: 'compact_summary' },
      ),
      createUserMessage("Now let's work on the authentication implementation", 'msg-1'),
      createAssistantMessage(
        textBlocks(
          "Based on our previous discussion, I'll implement JWT authentication with the PostgreSQL database.",
        ),
        'msg-2',
      ),
      createAssistantMessage(
        textBlocks(
          '## Implementation Plan Summary\n\n' +
            '1. Create user model\n' +
            '2. Implement JWT service\n' +
            '3. Add auth middleware\n' +
            '4. Create login/logout endpoints',
        ),
        'summary-2',
        { messageType: 'compact_summary' },
      ),
      createUserMessage('Looks good! Please proceed with the implementation.', 'msg-3'),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: "I'll start implementing the authentication system now.",
          },
          {
            type: 'tool_use',
            toolUseId: 'write-1',
            toolName: 'Write',
            toolInput: {
              file_path: '/src/auth/jwt.service.ts',
              content: 'JWT service implementation...',
            },
          },
        ],
        'msg-4',
        {
          toolResults: [{ toolUseId: 'write-1', content: 'File created successfully' }],
        },
      ),
    ],
    isLoading: false,
  },
}

export const MixedHiddenAndNormal: Story = {
  args: {
    messages: [
      createUserMessage('I need help debugging my application', 'msg-1'),
      createUserMessage('System context: Node.js v18, TypeScript 5.0, Express 4.18', 'hidden-1', {
        messageType: 'transcript_only',
      }),
      createAssistantMessage(
        textBlocks("I'll help you debug. Let me first check your project structure."),
        'msg-2',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            toolUseId: 'glob-1',
            toolName: 'Glob',
            toolInput: { pattern: '**/*.ts' },
          },
        ],
        'msg-3',
        {
          toolResults: [{ toolUseId: 'glob-1', content: 'Found 25 TypeScript files' }],
        },
      ),
      createAssistantMessage(
        textBlocks(
          'Internal analysis: The project appears to be a standard Express application with potential issues in error handling.',
        ),
        'hidden-2',
        { messageType: 'transcript_only' },
      ),
      createUserMessage('The app crashes when I try to authenticate', 'msg-4'),
      createAssistantMessage(
        [
          {
            type: 'text',
            text: 'Let me check the authentication code for potential issues.',
          },
          {
            type: 'tool_use',
            toolUseId: 'read-1',
            toolName: 'Read',
            toolInput: { file_path: '/src/auth/index.ts' },
          },
        ],
        'msg-5',
        {
          toolResults: [{ toolUseId: 'read-1', content: 'export * from "./authenticate"' }],
        },
      ),
      createAssistantMessage(
        textBlocks(
          '## Debug Session Summary\n\n' +
            'Issue identified: Missing error handler in auth middleware\n' +
            'Root cause: Uncaught promise rejection\n' +
            'Fix: Add try-catch wrapper',
        ),
        'summary-1',
        { messageType: 'compact_summary' },
      ),
      createAssistantMessage(
        textBlocks(
          "I found the issue! The authentication middleware is missing error handling. Here's how to fix it:\n\n" +
            '```typescript\n' +
            'try {\n' +
            '  await authenticate(req, res, next);\n' +
            '} catch (error) {\n' +
            '  next(error);\n' +
            '}\n' +
            '```',
        ),
        'msg-6',
      ),
    ],
    isLoading: false,
  },
}
