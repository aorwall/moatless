import type {
  AssistantMessage,
  AssistantMessageContent,
  FileSearchResult,
  Message,
  UploadFileResponse,
  UserMessage,
} from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react'
import { fn } from '@storybook/test'
import { Brain, Sparkles } from 'lucide-react'
import { useRef, useState } from 'react'
import { ChatInput, Files, Selectors, SendButton, UploadButton } from '../chat-input'
import { SelectorButton } from '../chat-input/selectors'
import { ChatView } from '../chat-view'
import { FileAutocomplete } from '../file-autocomplete'
import { MessageHistory } from '../message-history'

const meta = {
  title: 'Chat/InteractiveChat',
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

export const BasicInteractive = {
  render: () => {
    const [message, setMessage] = useState('')
    const [messages, setMessages] = useState<Message[]>([
      createUserMessage('Hello!', 'msg-1'),
      createAssistantMessage('Hi! How can I help you today?', 'msg-2'),
    ])
    const [messageIdCounter, setMessageIdCounter] = useState(3)
    const [isRunning, setIsRunning] = useState(false)

    const handleSubmit = () => {
      if (!message.trim()) return

      fn()(message)

      // Add user message
      const userMessage = createUserMessage(message, `msg-${messageIdCounter}`)
      setMessages((prev) => [...prev, userMessage])
      setMessageIdCounter((prev) => prev + 1)
      setMessage('')

      // Show thinking indicator
      setIsRunning(true)

      // Simulate assistant response after delay
      setTimeout(() => {
        const responses = [
          'I understand. Let me help you with that.',
          "That's a great question! Let me think about that.",
          "I'll look into that for you.",
          'Sure, I can help with that.',
          'Let me assist you with that task.',
        ]
        const randomResponse = responses[Math.floor(Math.random() * responses.length)]
        const assistantMessage = createAssistantMessage(randomResponse, `msg-${messageIdCounter}`)

        setMessages((prev) => [...prev, assistantMessage])
        setMessageIdCounter((prev) => prev + 1)
        setIsRunning(false)
      }, 1000)
    }

    return (
      <ChatView>
        <MessageHistory messages={messages} isRunning={isRunning} />
        <ChatInput
          message={message}
          onMessageChange={(msg) => {
            setMessage(msg)
            action('onMessageChange')(msg)
          }}
          onSubmit={handleSubmit}
          placeholder="Type your message..."
        >
          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <SelectorButton icon={Brain} label="Sonnet 4" />
              <SelectorButton icon={Sparkles} label="Edit Mode" />
            </Selectors>
            <SendButton onSubmit={handleSubmit} disabled={!message.trim()} />
          </div>
        </ChatInput>
      </ChatView>
    )
  },
} as unknown as Story

export const EmptyChat = {
  render: () => {
    const [message, setMessage] = useState('')
    const [messages, setMessages] = useState<Message[]>([])
    const [messageIdCounter, setMessageIdCounter] = useState(1)
    const [isRunning, setIsRunning] = useState(false)

    const handleSubmit = () => {
      if (!message.trim()) return

      fn()(message)

      // Add user message
      const userMessage = createUserMessage(message, `msg-${messageIdCounter}`)
      setMessages((prev) => [...prev, userMessage])
      setMessageIdCounter((prev) => prev + 1)
      setMessage('')

      // Show thinking indicator
      setIsRunning(true)

      // Simulate assistant response after delay
      setTimeout(() => {
        const responses = [
          'Hello! How can I assist you today?',
          "Hi there! I'm ready to help.",
          'Welcome! What can I do for you?',
          "Hey! I'm here to help you with your tasks.",
        ]
        const randomResponse = responses[Math.floor(Math.random() * responses.length)]
        const assistantMessage = createAssistantMessage(randomResponse, `msg-${messageIdCounter}`)

        setMessages((prev) => [...prev, assistantMessage])
        setMessageIdCounter((prev) => prev + 1)
        setIsRunning(false)
      }, 1000)
    }

    return (
      <ChatView>
        <MessageHistory messages={messages} isRunning={isRunning} />
        <ChatInput
          message={message}
          onMessageChange={(msg) => {
            setMessage(msg)
            action('onMessageChange')(msg)
          }}
          onSubmit={handleSubmit}
          placeholder="Start a conversation..."
        >
          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <SelectorButton icon={Brain} label="Sonnet 4" />
            </Selectors>
            <SendButton onSubmit={handleSubmit} disabled={!message.trim()} />
          </div>
        </ChatInput>
      </ChatView>
    )
  },
} as unknown as Story

export const WithFiles = {
  render: () => {
    const [message, setMessage] = useState('')
    const [messages, setMessages] = useState<Message[]>([
      createUserMessage('Hello!', 'msg-1'),
      createAssistantMessage('Hi! I see you have some files. How can I help?', 'msg-2'),
    ])
    const [messageIdCounter, setMessageIdCounter] = useState(3)
    const [isRunning, setIsRunning] = useState(false)
    const [files, setFiles] = useState(mockFiles)

    const handleSubmit = () => {
      if (!message.trim()) return

      fn()(message)

      // Add user message
      const userMessage = createUserMessage(message, `msg-${messageIdCounter}`)
      setMessages((prev) => [...prev, userMessage])
      setMessageIdCounter((prev) => prev + 1)
      setMessage('')

      // Show thinking indicator
      setIsRunning(true)

      // Simulate assistant response after delay
      setTimeout(() => {
        const responses = [
          'I understand. Let me help you with that.',
          "That's a great question! Let me think about that.",
          "I'll look into that for you.",
          'Sure, I can help with that.',
          'Let me assist you with that task.',
        ]
        const randomResponse = responses[Math.floor(Math.random() * responses.length)]
        const assistantMessage = createAssistantMessage(randomResponse, `msg-${messageIdCounter}`)

        setMessages((prev) => [...prev, assistantMessage])
        setMessageIdCounter((prev) => prev + 1)
        setIsRunning(false)
      }, 1000)
    }

    return (
      <ChatView>
        <MessageHistory messages={messages} isRunning={isRunning} />
        <ChatInput
          message={message}
          onMessageChange={(msg) => {
            setMessage(msg)
            action('onMessageChange')(msg)
          }}
          onSubmit={handleSubmit}
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
              <SendButton onSubmit={handleSubmit} disabled={!message.trim()} />
            </div>
          </div>
        </ChatInput>
      </ChatView>
    )
  },
} as unknown as Story

export const WithFileAutocomplete = {
  render: () => {
    const [message, setMessage] = useState('')
    const [messages, setMessages] = useState<Message[]>([
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
    ])
    const [messageIdCounter, setMessageIdCounter] = useState(5)
    const [isRunning, setIsRunning] = useState(false)
    const [isAutocompleteOpen, setIsAutocompleteOpen] = useState(false)
    const [autocompleteResults, setAutocompleteResults] = useState<FileSearchResult[]>([])
    const [selectedIndex, setSelectedIndex] = useState(0)
    const textareaRef = useRef<HTMLTextAreaElement>(null)

    // Mock file search results
    const mockFileResults: FileSearchResult[] = [
      {
        path: 'src/auth/authenticate.ts',
        absolutePath: '/project/src/auth/authenticate.ts',
        score: 0.95,
      },
      {
        path: 'src/auth/auth-provider.tsx',
        absolutePath: '/project/src/auth/auth-provider.tsx',
        score: 0.92,
      },
      {
        path: 'src/auth/auth-utils.ts',
        absolutePath: '/project/src/auth/auth-utils.ts',
        score: 0.9,
      },
      {
        path: 'src/hooks/use-auth.ts',
        absolutePath: '/project/src/hooks/use-auth.ts',
        score: 0.88,
      },
    ]

    const handleMessageChange = (msg: string) => {
      setMessage(msg)
      action('onMessageChange')(msg)

      // Detect @ pattern for file autocomplete
      if (textareaRef.current) {
        const cursorPosition = textareaRef.current.selectionStart || 0
        const textBeforeCursor = msg.substring(0, cursorPosition)
        const lastAtIndex = textBeforeCursor.lastIndexOf('@')

        if (lastAtIndex !== -1) {
          const textAfterAt = msg.substring(lastAtIndex + 1, cursorPosition)
          const spaceIndex = textAfterAt.indexOf(' ')

          if (spaceIndex === -1 && textAfterAt.length >= 2) {
            // Show autocomplete
            setIsAutocompleteOpen(true)
            setAutocompleteResults(mockFileResults)
            setSelectedIndex(0)
          } else {
            setIsAutocompleteOpen(false)
          }
        } else {
          setIsAutocompleteOpen(false)
        }
      }
    }

    const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!isAutocompleteOpen || autocompleteResults.length === 0) {
        return
      }

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setSelectedIndex((prev) => Math.min(prev + 1, autocompleteResults.length - 1))
          break
        case 'ArrowUp':
          e.preventDefault()
          setSelectedIndex((prev) => Math.max(prev - 1, 0))
          break
        case 'Enter':
          if (autocompleteResults[selectedIndex]) {
            e.preventDefault()
            selectFile(autocompleteResults[selectedIndex])
          }
          break
        case 'Escape':
          e.preventDefault()
          setIsAutocompleteOpen(false)
          break
        case 'Tab':
          if (autocompleteResults[selectedIndex]) {
            e.preventDefault()
            selectFile(autocompleteResults[selectedIndex])
          }
          break
      }
    }

    const selectFile = (file: FileSearchResult) => {
      action('onFileSelect')(file)

      // Replace @searchTerm with @filepath
      const cursorPosition = textareaRef.current?.selectionStart || 0
      const textBeforeCursor = message.substring(0, cursorPosition)
      const lastAtIndex = textBeforeCursor.lastIndexOf('@')
      const before = message.substring(0, lastAtIndex)
      const after = message.substring(cursorPosition)
      const newMessage = `${before}@${file.path}${after}`

      setMessage(newMessage)
      setIsAutocompleteOpen(false)

      // Restore focus
      setTimeout(() => {
        if (textareaRef.current) {
          const newCursorPosition = lastAtIndex + file.path.length + 1
          textareaRef.current.setSelectionRange(newCursorPosition, newCursorPosition)
          textareaRef.current.focus()
        }
      }, 0)
    }

    const handleSubmit = () => {
      if (!message.trim()) return

      fn()(message)

      // Add user message
      const userMessage = createUserMessage(message, `msg-${messageIdCounter}`)
      setMessages((prev) => [...prev, userMessage])
      setMessageIdCounter((prev) => prev + 1)
      setMessage('')

      // Show thinking indicator
      setIsRunning(true)

      // Simulate assistant response after delay
      setTimeout(() => {
        const responses = [
          'I understand. Let me help you with that.',
          "That's a great question! Let me think about that.",
          "I'll look into that for you.",
          'Sure, I can help with that.',
          'Let me assist you with that task.',
        ]
        const randomResponse = responses[Math.floor(Math.random() * responses.length)]
        const assistantMessage = createAssistantMessage(randomResponse, `msg-${messageIdCounter}`)

        setMessages((prev) => [...prev, assistantMessage])
        setMessageIdCounter((prev) => prev + 1)
        setIsRunning(false)
      }, 1000)
    }

    return (
      <ChatView>
        <MessageHistory messages={messages} isRunning={isRunning} />
        <ChatInput
          message={message}
          onMessageChange={handleMessageChange}
          onSubmit={handleSubmit}
          placeholder="Type your message... (use @auth to see file autocomplete)"
          textareaRef={textareaRef}
          onKeyDownCapture={handleKeyDown}
          autocompleteSlot={
            isAutocompleteOpen && (
              <FileAutocomplete
                results={autocompleteResults}
                selectedIndex={selectedIndex}
                onSelect={selectFile}
                isLoading={false}
                error={null}
                position="above"
              />
            )
          }
        >
          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <SelectorButton icon={Brain} label="Sonnet 4" />
              <SelectorButton icon={Sparkles} label="Edit Mode" />
            </Selectors>
            <SendButton onSubmit={handleSubmit} disabled={!message.trim()} />
          </div>
        </ChatInput>
      </ChatView>
    )
  },
} as unknown as Story
