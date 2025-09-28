import type { UploadFileResponse } from '@moatless/types'
import { action } from '@storybook/addon-actions'
import type { Meta, StoryObj } from '@storybook/react'
import { Brain, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { ChatInput, Files, Selectors, UploadButton } from '../index'

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
}: Partial<React.ComponentProps<typeof ChatInput>>) {
  const [message, setMessage] = useState(props.message || '')

  return (
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
      {...props}
    >
      {children}
    </ChatInput>
  )
}

export const Default: Story = {
  render: () => <InteractiveWrapper />,
}

export const WithMessage: Story = {
  render: () => <InteractiveWrapper message="Hello, can you help me with this code?" />,
}

export const WithPlaceholder: Story = {
  render: () => <InteractiveWrapper placeholder="Ask me anything about your codebase..." />,
}

export const Loading: Story = {
  render: () => <InteractiveWrapper message="Sending message..." loading={true} disabled={true} />,
}

export const Running: Story = {
  render: () => (
    <InteractiveWrapper
      message="Type your next request..."
      running={true}
      buttonText="Running"
      placeholder="Agent is running... Type your next request"
    />
  ),
}

export const Disabled: Story = {
  render: () => <InteractiveWrapper disabled={true} placeholder="Input is disabled" />,
}

export const WithFiles: Story = {
  render: () => {
    const [files, setFiles] = useState(mockFiles)
    return (
      <InteractiveWrapper>
        <Files
          files={files}
          onRemove={(id) => setFiles((prev) => prev.filter((f) => f.id !== id))}
        />
      </InteractiveWrapper>
    )
  },
}

export const WithSelectors: Story = {
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
}

export const LongText: Story = {
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
}

export const AllFeatures: Story = {
  render: () => {
    const [files, setFiles] = useState(mockFiles)
    return (
      <InteractiveWrapper
        running={true}
        buttonText="Running"
        placeholder="Agent is running... Type your next request"
      >
        <Files
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
              action('onFileSelect')(file)
            }}
          />
        </div>
      </InteractiveWrapper>
    )
  },
}

export const MinimalEmpty: Story = {
  render: () => <InteractiveWrapper placeholder="Start typing..." buttonText="Send" />,
}
