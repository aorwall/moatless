import type { UiContentBlock, UiMessage, UploadResponse } from '@moatless/api-client'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Brain, Sparkles } from 'lucide-react'
import {
  ChatInput,
  FileAttachments,
  Selectors,
  SendButton,
  StopButton,
  UploadButton,
} from '../chat-input'
import { SelectorButton } from '../chat-input/selectors'
import { ChatView } from '../chat-view'
import { MessageHistory } from '../message-history'
import { QueuedMessages } from '../queued-messages'

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

const createUserMessage = (content: string, id: string, isQueued?: boolean): UiMessage => ({
  uuid: id,
  role: 'user',
  content: [{ type: 'text', text: content }],
  toolResults: [],
  ...(isQueued ? { isQueued: true } : {}),
})

const createAssistantMessage = (content: UiContentBlock[], id: string): UiMessage => ({
  uuid: id,
  role: 'assistant',
  content,
  toolResults: [],
})

const textBlocks = (text: string): UiContentBlock[] => [{ type: 'text', text }]

const mockFiles: UploadResponse[] = [
  {
    fileId: 'file-1',
    filename: 'screenshot.png',
    mimeType: 'image/png',
    size: 125000,
  },
  {
    fileId: 'file-2',
    filename: 'code-example.ts',
    mimeType: 'text/typescript',
    size: 3500,
  },
]

// Noop handlers for static stories
const noop = () => undefined

export const Default = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={[
          createUserMessage('Hello!', 'msg-1'),
          createAssistantMessage(textBlocks('Hi! How can I help you today?'), 'msg-2'),
          createUserMessage('I need help implementing a new feature', 'msg-3'),
          createAssistantMessage(
            textBlocks(
              "I'd be happy to help! Can you tell me more about the feature you want to implement?",
            ),
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
                toolUseId: 'grep-1',
                toolName: 'Grep',
                toolInput: { pattern: 'auth', path: '/src' },
              },
              {
                type: 'text',
                text: 'I found some existing authentication code. Let me review it and suggest improvements.',
              },
            ],
            'msg-6',
          ),
        ]}
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
        messages={[
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
          ),
        ]}
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
        messages={[createUserMessage('What is the capital of France?', 'msg-1')]}
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

/**
 * Shows queued messages stacked above the chat input while the agent is running.
 * Each queued message has an X button to remove it.
 */
export const WithQueuedMessages = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={[
          createUserMessage('Add authentication to the app', 'msg-1'),
          createAssistantMessage(
            [
              {
                type: 'text',
                text: "I'll implement authentication for your app. Let me start by examining the project structure.",
              },
              {
                type: 'tool_use',
                toolUseId: 'tool-1',
                toolName: 'Glob',
                toolInput: { pattern: 'src/**/*.ts' },
              },
            ],
            'msg-2',
          ),
          createUserMessage('Also add rate limiting to the login endpoint', 'msg-3', true),
          createUserMessage('And please use bcrypt for password hashing', 'msg-4', true),
        ]}
        isRunning
      />
      <QueuedMessages
        messages={[
          createUserMessage('Also add rate limiting to the login endpoint', 'msg-3', true),
          createUserMessage('And please use bcrypt for password hashing', 'msg-4', true),
        ]}
        onRemove={noop}
      />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Task is running... Type your next request"
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
            <SelectorButton icon={Sparkles} label="Edit Mode" />
          </Selectors>
          <div className="flex items-center gap-2">
            <StopButton onClick={noop} />
            <SendButton disabled={false} />
          </div>
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story

/**
 * Shows a single queued message above the chat input — the minimal case.
 */
export const WithSingleQueuedMessage = {
  render: () => (
    <ChatView>
      <MessageHistory
        messages={[
          createUserMessage('Fix the button styling on the homepage', 'msg-1'),
          createAssistantMessage(textBlocks('Working on it now...'), 'msg-2'),
          createUserMessage(
            'Oh and also update the footer links while you are at it',
            'msg-3',
            true,
          ),
        ]}
        isRunning
      />
      <QueuedMessages
        messages={[
          createUserMessage(
            'Oh and also update the footer links while you are at it',
            'msg-3',
            true,
          ),
        ]}
        onRemove={noop}
      />
      <ChatInput
        message=""
        onMessageChange={noop}
        onSubmit={noop}
        placeholder="Task is running... Type your next request"
      >
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <SelectorButton icon={Brain} label="Sonnet 4" />
          </Selectors>
          <div className="flex items-center gap-2">
            <StopButton onClick={noop} />
            <SendButton disabled={false} />
          </div>
        </div>
      </ChatInput>
    </ChatView>
  ),
} as unknown as Story
