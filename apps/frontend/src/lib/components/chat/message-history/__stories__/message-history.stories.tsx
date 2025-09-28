import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UserMessage,
} from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react'
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
const createUserMessage = (content: string, id: string): UserMessage => ({
  id,
  role: 'user',
  content,
  timestamp: new Date(),
})

const createAssistantMessage = (
  content: string | AssistantMessageContent[],
  id: string,
): AssistantMessage => ({
  id,
  role: 'assistant',
  content,
  timestamp: new Date(),
})

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
        "Of course! I'd be happy to help you with authentication. What specific aspect are you working on?",
        'msg-2',
      ),
      createUserMessage('I need to implement JWT token refresh', 'msg-3'),
      createAssistantMessage(
        "I'll help you implement JWT token refresh. Let me first check your current authentication setup.",
        'msg-4',
      ),
    ] as Message[],
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
            tool_use_id: 'tool-1',
            tool_name: 'Read',
            tool_input: {
              file_path: '/src/auth/authenticate.js',
            },
            tool_result: {
              content: 'function authenticate(user) {\n  return token.verify(user.token);\n}',
            },
          },
          {
            type: 'text',
            text: 'I found the authentication function. It appears to be missing error handling.',
          },
        ],
        'msg-2',
      ),
    ] as Message[],
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
            tool_use_id: 'grep-1',
            tool_name: 'Grep',
            tool_input: { pattern: 'authenticate', path: '/src' },
            tool_result: { content: 'Found 5 matches' },
          },
        ],
        'msg-2',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'read-1',
            tool_name: 'Read',
            tool_input: { file_path: '/src/auth.js' },
            tool_result: { content: 'auth.js content' },
          },
        ],
        'msg-3',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'read-2',
            tool_name: 'Read',
            tool_input: { file_path: '/src/token.js' },
            tool_result: { content: 'token.js content' },
          },
        ],
        'msg-4',
      ),
      createAssistantMessage(
        'Found all the authentication files. They need some refactoring.',
        'msg-5',
      ),
    ] as Message[],
    isLoading: false,
  },
}

export const LongConversation: Story = {
  args: {
    messages: [
      createUserMessage('Hello, I need help with authentication', 'msg-1'),
      createAssistantMessage(
        'Hi! I can help you with authentication. What issue are you facing?',
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
            tool_use_id: 'read-1',
            tool_name: 'Read',
            tool_input: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts',
            },
            tool_result: {
              content:
                'export class TokenValidationService {\n  validate(token: string) { /* ... */ }\n}',
            },
          },
        ],
        'msg-4',
      ),
      createUserMessage('Can you also check the refresh token service?', 'msg-5'),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'read-2',
            tool_name: 'Read',
            tool_input: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/refresh-token/implementations/RefreshTokenService.ts',
            },
            tool_result: {
              content:
                'export class RefreshTokenService {\n  refresh(token: string) { /* ... */ }\n}',
            },
          },
        ],
        'msg-6',
      ),
      createUserMessage('What did you find?', 'msg-7'),
      createAssistantMessage(
        'Both services look good. The token validation is working correctly, and the refresh mechanism is properly implemented.',
        'msg-8',
      ),
      createUserMessage(
        'Here is my config file:\n\n```json\n{\n  "tokenValidator": "/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts",\n  "refreshService": "/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/refresh-token/implementations/RefreshTokenService.ts"\n}\n```',
        'msg-9',
      ),
      createAssistantMessage(
        'I see the issue - those paths should be relative. Let me fix that for you.',
        'msg-10',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'edit-1',
            tool_name: 'Edit',
            tool_input: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/configuration/services.config.json',
              old_string:
                '"/Users/developer/projects/enterprise-app/backend/src/modules/authentication/services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts"',
              new_string:
                '"./services/token-management/jwt-validator/implementations/v2/TokenValidationService.ts"',
            },
            tool_result: {
              content: 'File updated successfully',
            },
          },
        ],
        'msg-11',
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
            tool_use_id: 'edit-2',
            tool_name: 'Edit',
            tool_input: {
              file_path:
                '/Users/developer/projects/enterprise-app/backend/src/modules/authentication/middleware/validation/token-expiry-check/ExpiryValidationMiddleware.ts',
              old_string: 'old code',
              new_string: 'new code',
            },
            tool_result: {
              content:
                'Successfully updated /Users/developer/projects/enterprise-app/backend/src/modules/authentication/middleware/validation/token-expiry-check/ExpiryValidationMiddleware.ts',
            },
          },
        ],
        'msg-13',
      ),
      createUserMessage('Thanks! Does this affect the hashing utility?', 'msg-14'),
      createAssistantMessage(
        "No, the hashing utility at `/Users/developer/projects/enterprise-app/backend/src/modules/authentication/utilities/crypto/hashing/algorithms/SHA256HashingUtility.ts` is independent and doesn't need changes.",
        'msg-15',
      ),
      createUserMessage('Perfect! One more thing - can you run the tests?', 'msg-16'),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'bash-1',
            tool_name: 'Bash',
            tool_input: {
              command: 'npm test',
            },
            tool_result: {
              content: 'Running tests...\n✓ All tests passed\n\nTest files: 25\nTests passed: 150',
            },
          },
        ],
        'msg-17',
      ),
      createUserMessage('Excellent! Everything is working now.', 'msg-18'),
      createAssistantMessage(
        'Great! All the authentication files have been updated and tests are passing. The configuration now uses relative paths which is much better for portability.',
        'msg-19',
      ),
    ] as Message[],
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
            tool_use_id: 'todo-1',
            tool_name: 'TodoWrite',
            tool_input: {
              todos: [
                { id: '1', content: 'Analyze authentication flow', status: 'completed' },
                { id: '2', content: 'Fix token initialization', status: 'completed' },
                { id: '3', content: 'Add error handling', status: 'in_progress' },
                { id: '4', content: 'Write unit tests', status: 'pending' },
                { id: '5', content: 'Update documentation', status: 'pending' },
              ],
            },
            tool_result: { content: 'Todos updated' },
          },
        ],
        'msg-2',
      ),
      createUserMessage('Great! How is the progress?', 'msg-3'),
      createAssistantMessage(
        "I've completed the analysis and fixed the token initialization. Currently working on adding error handling.",
        'msg-4',
      ),
    ] as Message[],
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
            tool_use_id: 'glob-1',
            tool_name: 'Glob',
            tool_input: { pattern: '**/auth*.{js,ts}' },
            tool_result: { content: 'No authentication files found' },
          },
          {
            type: 'text',
            text: "I see you don't have any authentication files yet. Let me create the structure:",
          },
        ],
        'msg-2',
      ),
      createAssistantMessage(
        [
          {
            type: 'tool_use',
            tool_use_id: 'todo-2',
            tool_name: 'TodoWrite',
            tool_input: {
              todos: [
                { id: '1', content: 'Create auth service', status: 'pending' },
                { id: '2', content: 'Implement JWT handling', status: 'pending' },
                { id: '3', content: 'Add refresh token logic', status: 'pending' },
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
            tool_use_id: 'write-1',
            tool_name: 'Write',
            tool_input: {
              file_path: '/src/auth/authService.js',
              content: 'class AuthService { ... }',
            },
            tool_result: { content: 'File created successfully' },
          },
        ],
        'msg-4',
      ),
      createUserMessage('Can you add error handling?', 'msg-5'),
      createAssistantMessage(
        '```javascript\n' +
          'try {\n' +
          '  const token = await authService.authenticate(credentials);\n' +
          '  return { success: true, token };\n' +
          '} catch (error) {\n' +
          '  console.error("Authentication failed:", error);\n' +
          '  return { success: false, error: error.message };\n' +
          '}\n' +
          '```',
        'msg-6',
      ),
    ] as Message[],
    isLoading: false,
  },
}

export const ThinkingState: Story = {
  args: {
    messages: [
      createUserMessage('Can you help me refactor the authentication module?', 'msg-1'),
      createAssistantMessage(
        "I'll help you refactor the authentication module. Let me first analyze the current code structure.",
        'msg-2',
      ),
      createUserMessage('Also check if there are any security issues', 'msg-3'),
    ] as Message[],
    isLoading: false,
    isRunning: true,
  },
}
