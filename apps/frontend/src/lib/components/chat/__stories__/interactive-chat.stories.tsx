import type {
  AssistantMessage,
  AssistantMessageContent,
  FileSearchResult,
  Message,
  UserMessage,
} from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react'
import { Brain, Sparkles } from 'lucide-react'
import { useRef, useState } from 'react'
import { action } from 'storybook/actions'
import { ChatInput, Selectors, SendButton } from '../chat-input'
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

export const CompleteExample = {
  render: () => {
    const [message, setMessage] = useState('')
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
          onMessageChange={handleMessageChange}
          onSubmit={() => {
            action('onSubmit')(message)
            setMessage('')
          }}
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
            <SendButton
              onSubmit={() => {
                action('onSubmit')(message)
                setMessage('')
              }}
              disabled={!message.trim()}
            />
          </div>
        </ChatInput>
      </ChatView>
    )
  },
} as unknown as Story
