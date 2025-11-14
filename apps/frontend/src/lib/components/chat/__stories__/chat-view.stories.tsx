import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UploadFileResponse,
  UserMessage,
} from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Brain, Sparkles } from 'lucide-react'
import { ChatInput, FileAttachments, Selectors, SendButton, UploadButton } from '../chat-input'
import { SelectorButton } from '../chat-input/selectors'
import { ChatView } from '../chat-view'
import { MessageHistory } from '../message-history'

const meta = {
  title: 'Chat/ChatView',
  component: ChatView,
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
} satisfies Meta<typeof ChatView>

export default meta

type Story = StoryObj<typeof meta>

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

const mockFiles: UploadFileResponse[] = [
  {
    id: 'file-1',
    filename: 'screenshot.png',
    path: 'screenshot.png',
    absolutePath: '/tmp/screenshot.png',
    mimeType: 'image/png',
    size: 125000,
    uploadedAt: new Date().toISOString(),
  },
  {
    id: 'file-2',
    filename: 'code-example.ts',
    path: 'code-example.ts',
    absolutePath: '/tmp/code-example.ts',
    mimeType: 'text/typescript',
    size: 3500,
    uploadedAt: new Date().toISOString(),
  },
]

// Noop handlers for static stories
const noop = () => undefined

export const Default = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={
          [
            createUserMessage('Hello!', 'msg-1'),
            createAssistantMessage('Hi! How can I help you today?', 'msg-2'),
            createUserMessage('I need help implementing a new feature', 'msg-3'),
            createAssistantMessage(
              "I'd be happy to help! Can you tell me more about the feature you want to implement?",
              'msg-4',
            ),
            createUserMessage('I want to add authentication to my app', 'msg-5'),
            createAssistantMessage(
              [
                {
                  type: 'text',
                  text: 'Great! Let me check your current setup.',
                },
                {
                  type: 'tool_use',
                  tool_use_id: 'grep-1',
                  tool_name: 'Grep',
                  tool_input: { pattern: 'auth', path: '/src' },
                  tool_result: {
                    content: 'Found 3 authentication-related files',
                  },
                },
                {
                  type: 'text',
                  text: 'I found some existing authentication code. Let me review it and suggest improvements.',
                },
              ],
              'msg-6',
            ),
          ] as Message[]
        }
      />
      <ChatInput message="" onMessageChange={noop} onSubmit={noop} placeholder="Type a message...">
        <FileAttachments files={mockFiles} onRemove={noop} />
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
            <SelectorButton icon={Sparkles} label="Edit Mode" />
          </Selectors>
          <div className="flex items-center gap-2">
            <UploadButton onFileSelect={noop} />
            <SendButton disabled />
          </div>
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

export const EmptyChat = {
  render: () => (
    <ChatView>
      <MessageHistory messages={[]} />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Start a conversation..."
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
            <SelectorButton icon={Sparkles} label="Edit Mode" />
          </Selectors>
          <SendButton disabled />
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

export const WithToolCalls = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={
          [
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
          ] as Message[]
        }
      />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Continue the conversation..."
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
          </Selectors>
          <SendButton disabled />
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

export const ErrorState = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={undefined}
        error={new Error('Failed to load messages from the server')}
      />
      <ChatInput message="" onMessageChange={noop} onSubmit={noop} placeholder="Type a message...">
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
          </Selectors>
          <SendButton disabled />
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

export const LoadingState = {
  render: () => (
    <ChatView>
      <MessageHistory messages={[]} isLoading />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Type a message..."
        loading
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
          </Selectors>
          <SendButton disabled />
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

export const ThinkingState = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={[createUserMessage('What is the capital of France?', 'msg-1')] as Message[]}
        isRunning
      />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Type a message..."
        loading
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
          </Selectors>
          <SendButton disabled />
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story
