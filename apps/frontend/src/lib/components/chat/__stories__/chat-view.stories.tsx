import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UploadFileResponse,
  UserMessage,
} from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react'
import { Brain, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { action } from 'storybook/actions'
import { ChatInput, Files, Selectors, SendButton, UploadButton } from '../chat-input'
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

function InteractiveChatView({
  messages,
  isLoading,
  error,
  children,
  buttonText = 'Send',
  running = false,
  loading = false,
  disabled = false,
  ...inputProps
}: {
  messages?: Message[]
  isLoading?: boolean
  error?: Error | null
  children?: React.ReactNode
  buttonText?: string
  running?: boolean
  loading?: boolean
  disabled?: boolean
} & Partial<
  Omit<React.ComponentProps<typeof ChatInput>, 'message' | 'onMessageChange' | 'onSubmit'>
>) {
  const [message, setMessage] = useState('')

  return (
    <ChatView>
      <MessageHistory messages={messages} isLoading={isLoading} error={error} />
      <ChatInput
        message={message}
        onMessageChange={(msg) => {
          setMessage(msg)
          action('onMessageChange')(msg)
        }}
        onSubmit={() => {
          action('onSubmit')(message)
          setMessage('')
        }}
        disabled={disabled}
        {...inputProps}
      >
        {children}
      </ChatInput>
    </ChatView>
  )
}

export const Default: Story = {
  render: () => {
    const [files, setFiles] = useState(mockFiles)
    const [message, setMessage] = useState('')
    return (
      <ChatView>
        <MessageHistory
          messages={
            [
              createUserMessage('Hello!', 'msg-1'),
              createAssistantMessage('Hi! How can I help you today?', 'msg-2'),
            ] as Message[]
          }
        />
        <ChatInput
          message={message}
          onMessageChange={(msg) => {
            setMessage(msg)
            action('onMessageChange')(msg)
          }}
          onSubmit={() => {
            action('onSubmit')(message)
            setMessage('')
          }}
          placeholder="Type your message..."
        >
          {files.length > 0 && (
            <Files
              files={files}
              onRemove={(id) => setFiles((prev) => prev.filter((f) => f.id !== id))}
            />
          )}
          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <SelectorButton icon={Brain} label="Sonnet 4" />
              <SelectorButton icon={Sparkles} label="Edit Mode" />
            </Selectors>
            <div className="flex items-center gap-2">
              <UploadButton
                onFileSelect={(file) => {
                  action('onFileSelect')(file)
                }}
              />
              <SendButton
                onSubmit={() => {
                  action('onSubmit')(message)
                  setMessage('')
                }}
                message={message}
              />
            </div>
          </div>
        </ChatInput>
      </ChatView>
    )
  },
}

export const EmptyChat: Story = {
  render: () => <InteractiveChatView messages={[]} placeholder="Start a conversation..." />,
}

export const WithToolCalls: Story = {
  render: () => (
    <InteractiveChatView
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
      placeholder="Continue the conversation..."
    />
  ),
}

export const ErrorState: Story = {
  render: () => (
    <InteractiveChatView
      messages={undefined}
      error={new Error('Failed to load messages from the server')}
      placeholder="Type a message..."
    />
  ),
}

export const CompleteExample: Story = {
  render: () => {
    const [message, setMessage] = useState('')
    return (
      <ChatView>
        <MessageHistory
          messages={
            [
              createUserMessage('Hello, can you help me?', 'msg-1'),
              createAssistantMessage('Of course! What do you need help with?', 'msg-2'),
              createUserMessage('I need to implement authentication', 'msg-3'),
              createAssistantMessage(
                [
                  {
                    type: 'text',
                    text: 'Let me help you with that. First, let me check your current setup.',
                  },
                  {
                    type: 'tool_use',
                    tool_use_id: 'grep-1',
                    tool_name: 'Grep',
                    tool_input: { pattern: 'auth', path: '/src' },
                    tool_result: { content: 'Found 5 authentication files' },
                  },
                ],
                'msg-4',
              ),
            ] as Message[]
          }
        />
        <ChatInput
          message={message}
          onMessageChange={(msg) => {
            setMessage(msg)
            action('onMessageChange')(msg)
          }}
          onSubmit={() => {
            action('onSubmit')(message)
            setMessage('')
          }}
          placeholder="Type your message..."
        >
          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <SelectorButton icon={Brain} label="Sonnet 4" />
              <SelectorButton icon={Sparkles} label="Edit Mode" />
            </Selectors>
            <SendButton
              onSubmit={() => {
                action('onSubmit')(message)
                setMessage('')
              }}
              message={message}
            />
          </div>
        </ChatInput>
      </ChatView>
    )
  },
}
