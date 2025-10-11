import type { UserMessage } from '@moatless/types'
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

const createUserMessage = (content: string, id = 'user-msg-1'): UserMessage => ({
  id,
  role: 'user',
  content,
  timestamp: new Date(),
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
