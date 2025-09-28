import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UploadFileResponse,
  UserMessage,
} from '@moatless/types'
import { action } from '@storybook/addon-actions'
import type { Meta, StoryObj } from '@storybook/react'
import { Brain, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { ChatInput, Files, Selectors, SendButton, UploadButton } from '../chat-input'
import { SelectorButton } from '../chat-input/selectors'
import { ChatView } from '../chat-view'
import { MessageHistory } from '../message-history'

const meta = {
  title: 'Chat/Interactive Chat',
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

export const InteractiveMockChat: Story = {
  render: () => {
    const [messages, setMessages] = useState<Message[]>([
      createUserMessage('Hello! Can you help me?', 'msg-1'),
      createAssistantMessage('Hi! Of course, I can help you. What do you need?', 'msg-2'),
    ])
    const [message, setMessage] = useState('')
    const [files, setFiles] = useState<UploadFileResponse[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [messageCounter, setMessageCounter] = useState(3)

    const mockResponses: (string | AssistantMessageContent[])[] = [
      'Sure, I can help with that. Let me check the code for you.',
      [
        {
          type: 'text',
          text: 'Let me read that file for you.',
        },
        {
          type: 'tool_use',
          tool_use_id: 'read-1',
          tool_name: 'Read',
          tool_input: {
            file_path: '/src/app.ts',
          },
          tool_result: {
            content: 'export function main() {\n  console.log("Hello World");\n}',
          },
        },
        {
          type: 'text',
          text: 'I found the main function. It logs "Hello World" to the console.',
        },
      ],
      "That makes sense! Is there anything else you'd like me to help with?",
      [
        {
          type: 'text',
          text: 'Let me run the tests for you.',
        },
        {
          type: 'tool_use',
          tool_use_id: 'bash-1',
          tool_name: 'Bash',
          tool_input: {
            command: 'npm test',
          },
          tool_result: {
            content: '✓ All tests passed\n\nTests: 15 passed, 15 total\nTime: 2.5s',
          },
        },
      ],
      'Great! All tests are passing. Your code looks good.',
    ]

    const handleSubmit = () => {
      if (!message.trim()) return

      const userMsg = createUserMessage(message, `msg-${messageCounter}`)
      setMessages((prev) => [...prev, userMsg])
      setMessage('')
      setIsLoading(true)
      setMessageCounter((c) => c + 1)
      action('user-message')(message)

      setTimeout(() => {
        const responseIndex = (messageCounter - 3) % mockResponses.length
        const response = mockResponses[responseIndex]
        const assistantMsg = createAssistantMessage(response, `msg-${messageCounter + 1}`)
        setMessages((prev) => [...prev, assistantMsg])
        setIsLoading(false)
        setMessageCounter((c) => c + 1)
        action('assistant-response')(response)
      }, 2000)
    }

    return (
      <ChatView>
        <MessageHistory messages={messages} isLoading={false} isRunning={isLoading} />
        <ChatInput
          message={message}
          onMessageChange={setMessage}
          onSubmit={handleSubmit}
          disabled={isLoading}
          placeholder={isLoading ? 'Waiting for response...' : 'Type your message...'}
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
                  const mockFile: UploadFileResponse = {
                    id: `file-${Date.now()}`,
                    filename: file.name,
                    path: file.name,
                    absolutePath: `/tmp/${file.name}`,
                    mimeType: file.type,
                    size: file.size,
                    uploadedAt: new Date().toISOString(),
                  }
                  setFiles((prev) => [...prev, mockFile])
                }}
                disabled={isLoading}
              />
              <SendButton
                onSubmit={handleSubmit}
                disabled={isLoading || !message.trim()}
                loading={isLoading}
                message={message}
              />
            </div>
          </div>
        </ChatInput>
      </ChatView>
    )
  },
}
