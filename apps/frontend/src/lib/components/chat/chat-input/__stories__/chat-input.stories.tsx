import type { UploadFileResponse } from '@moatless/types'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Brain, Sparkles } from 'lucide-react'
import type React from 'react'
import { useState } from 'react'
import { fn } from 'storybook/test'
import { Button } from '@/lib/components/ui/button'

import { ChatInput, FileAttachments, Selectors, UploadButton } from '../index'

const meta = {
  title: 'Chat/ChatInput',
  component: ChatInput,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="w-[600px] bg-background p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ChatInput>

export default meta
type Story = StoryObj<typeof meta>

const MockSelector = ({ icon: Icon, label }: { icon: any; label: string }) => (
  <Button variant="ghost" size="sm" className="h-6 px-2 gap-1">
    <Icon className="h-3 w-3" />
    <span className="text-xs">{label}</span>
  </Button>
)

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

function InteractiveWrapper({
  children,
  ...props
}: Partial<React.ComponentProps<typeof ChatInput>> & {
  disabled?: boolean
  running?: boolean
  buttonText?: string
}) {
  const [message, setMessage] = useState(props.message || '')
  // Remove invalid props that don't exist on ChatInput
  const {
    disabled: _disabled,
    running: _running,
    buttonText: _buttonText,
    ...chatInputProps
  } = props

  return (
    <ChatInput
      message={message}
      onMessageChange={(msg) => {
        setMessage(msg)
        fn()(msg)
      }}
      onSubmit={() => {
        fn()(message)
        setMessage('')
      }}
      {...chatInputProps}
    >
      {children}
    </ChatInput>
  )
}

export const Default = {
  render: () => <InteractiveWrapper />,
} as unknown as Story

export const WithMessage = {
  render: () => <InteractiveWrapper message="Hello, can you help me with this code?" />,
} as unknown as Story

export const WithPlaceholder = {
  render: () => <InteractiveWrapper placeholder="Ask me anything about your codebase..." />,
} as unknown as Story

export const Loading = {
  render: () => <InteractiveWrapper message="Sending message..." loading={true} disabled={true} />,
} as unknown as Story

export const Running = {
  render: () => (
    <InteractiveWrapper
      message="Type your next request..."
      running={true}
      buttonText="Running"
      placeholder="Agent is running... Type your next request"
    />
  ),
} as unknown as Story

export const Disabled = {
  render: () => <InteractiveWrapper disabled={true} placeholder="Input is disabled" />,
} as unknown as Story

export const WithFiles = {
  render: () => {
    const [files, setFiles] = useState(mockFiles)
    return (
      <InteractiveWrapper>
        <FileAttachments
          files={files}
          onRemove={(id) => setFiles((prev) => prev.filter((f) => f.id !== id))}
        />
      </InteractiveWrapper>
    )
  },
} as unknown as Story

export const WithSelectors = {
  render: () => (
    <InteractiveWrapper>
      <div className="flex items-center justify-between gap-2">
        <Selectors>
          <MockSelector icon={Brain} label="Sonnet 4" />
          <MockSelector icon={Sparkles} label="Edit Mode" />
        </Selectors>
      </div>
    </InteractiveWrapper>
  ),
} as unknown as Story

export const LongText = {
  render: () => (
    <InteractiveWrapper
      message={`This is a very long message that demonstrates the auto-resize functionality of the chat input.

As you type more lines, the textarea will automatically expand up to the maximum height.

Here's some sample code:
function example() {
  console.log("Hello World");
  return true;
}

This helps test the scrolling behavior when content exceeds the maximum height.`}
      className="max-h-[150px]"
    />
  ),
} as unknown as Story

export const AllFeatures = {
  render: () => {
    const [files, setFiles] = useState(mockFiles)
    return (
      <InteractiveWrapper
        running={true}
        buttonText="Running"
        placeholder="Agent is running... Type your next request"
      >
        <FileAttachments
          files={files}
          onRemove={(id) => setFiles((prev) => prev.filter((f) => f.id !== id))}
        />
        <div className="flex items-center justify-between gap-2">
          <Selectors>
            <MockSelector icon={Brain} label="Sonnet 4" />
            <MockSelector icon={Sparkles} label="Edit Mode" />
          </Selectors>
          <UploadButton
            onFileSelect={(file) => {
              fn()(file)
            }}
          />
        </div>
      </InteractiveWrapper>
    )
  },
} as unknown as Story

export const MinimalEmpty = {
  args: {},
  render: () => <InteractiveWrapper placeholder="Start typing..." buttonText="Send" />,
} as unknown as Story
