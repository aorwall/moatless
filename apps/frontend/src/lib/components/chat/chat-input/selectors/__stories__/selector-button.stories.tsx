import type { Meta, StoryObj } from '@storybook/react'
import { fn } from '@storybook/test'
import { Bot, Brain, Database, Sparkles } from 'lucide-react'
import { SelectorButton } from '../selector-button'

const meta = {
  title: 'Chat/Selectors/SelectorButton',
  component: SelectorButton,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="w-[400px] p-4 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SelectorButton>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  args: {
    icon: Brain,
    label: 'Sonnet 4',
    onClick: fn(),
  },
}

export const ModelSelector: Story = {
  args: {
    icon: Bot,
    label: 'Claude Opus 4',
    onClick: fn(),
  },
}

export const RepositorySelector: Story = {
  args: {
    icon: Database,
    label: 'my-project',
    onClick: fn(),
  },
}

export const Disabled: Story = {
  args: {
    icon: Brain,
    label: 'Disabled',
    disabled: true,
    onClick: fn(),
  },
}

export const LongLabel: Story = {
  args: {
    icon: Database,
    label: 'very-long-repository-name-that-should-truncate',
    onClick: fn(),
  },
}

export const MultipleSelectors: Story = {
  args: {},
  render: () => (
    <div className="flex items-center gap-2">
      <SelectorButton icon={Brain} label="Sonnet 4" onClick={fn()} />
      <SelectorButton icon={Sparkles} label="Edit Mode" onClick={fn()} />
      <SelectorButton icon={Database} label="my-project" onClick={fn()} />
    </div>
  ),
}
