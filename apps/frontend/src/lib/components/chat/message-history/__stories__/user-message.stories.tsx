import type { UiMessage } from '@moatless/api-client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { UserMessageComponent } from '../user-message.tsx'

const meta = {
  title: 'Chat/Messages/User Message',
  component: UserMessageComponent,
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
} satisfies Meta<typeof UserMessageComponent>

export default meta
type Story = StoryObj<typeof meta>

const createUserMessage = (
  content: string,
  id = 'user-msg-1',
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

export const SimpleText: Story = {
  args: {
    message: createUserMessage('Can you help me fix the authentication bug?'),
  },
}

export const LongText: Story = {
  args: {
    message: createUserMessage(
      'I need help with several issues in the authentication flow:\n\n' +
        '1. Users are getting logged out randomly after about 15 minutes\n' +
        '2. The refresh token mechanism seems to be failing\n' +
        '3. Some users report seeing "Invalid token" errors\n\n' +
        'Can you investigate these issues and provide a solution?',
    ),
  },
}

export const WithMarkdown: Story = {
  args: {
    message: createUserMessage(
      '## Bug Report\n\n' +
        'The following code is causing issues:\n\n' +
        '```javascript\n' +
        'function authenticate(user) {\n' +
        '  return token.verify(user.token);\n' +
        '}\n' +
        '```\n\n' +
        '**Error:** `token is undefined`\n\n' +
        'Please help me fix this.',
    ),
  },
}

export const WithLinks: Story = {
  args: {
    message: createUserMessage(
      'Check out the documentation at https://example.com/docs and the ' +
        'issue tracker at [GitHub](https://github.com/example/repo/issues).',
    ),
  },
}

export const ComplexFormatting: Story = {
  args: {
    message: createUserMessage(
      '# Project Requirements\n\n' +
        '## Overview\n' +
        'We need to implement **authentication** with the following features:\n\n' +
        '### Required Features\n' +
        '1. **Login/Logout** - Basic authentication flow\n' +
        '2. **Token Refresh** - Auto-refresh tokens\n' +
        '3. **Session Management** - Track active sessions\n\n' +
        '### Code Example\n' +
        '```typescript\n' +
        'interface AuthConfig {\n' +
        '  tokenLifetime: number;\n' +
        '  refreshInterval: number;\n' +
        '}\n' +
        '```\n\n' +
        '### Timeline\n' +
        '- Week 1: Design API\n' +
        '- Week 2: Implementation\n' +
        '- Week 3: Testing\n\n' +
        '> **Note:** Security review required before deployment\n\n' +
        'For more details, see [RFC-001](https://example.com/rfc/001).',
    ),
  },
}

export const HiddenMessage: Story = {
  args: {
    message: createUserMessage(
      'This is a hidden message that is only visible in the transcript. It contains context that was provided to the agent but should not be shown prominently in the chat view.',
      'hidden-msg-1',
      { messageType: 'transcript_only' },
    ),
  },
}

export const SummaryMessage: Story = {
  args: {
    message: createUserMessage(
      '## Conversation Summary\n\n' +
        'The user discussed:\n' +
        '- Authentication improvements\n' +
        '- Token refresh mechanisms\n' +
        '- Error handling patterns\n\n' +
        'Key decisions made:\n' +
        '1. Use JWT with 15-minute expiry\n' +
        '2. Implement silent token refresh\n' +
        '3. Add comprehensive logging',
      'summary-msg-1',
      { messageType: 'compact_summary' },
    ),
  },
}

export const HiddenWithLongContent: Story = {
  args: {
    message: createUserMessage(
      'This is a much longer hidden message that contains extensive context information.\n\n' +
        '## Background Context\n' +
        'The project uses a microservices architecture with the following services:\n' +
        '- Auth Service (handles authentication)\n' +
        '- User Service (manages user data)\n' +
        '- Gateway Service (API gateway)\n\n' +
        '## Current Issues\n' +
        '1. Token validation is inconsistent\n' +
        '2. Refresh tokens expire too quickly\n' +
        '3. Session management needs improvement\n\n' +
        '```typescript\n' +
        'interface TokenPayload {\n' +
        '  userId: string;\n' +
        '  exp: number;\n' +
        '  iat: number;\n' +
        '}\n' +
        '```',
      'hidden-long-msg-1',
      { messageType: 'transcript_only' },
    ),
  },
}
