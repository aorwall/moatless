import { action } from '@storybook/addon-actions'
import type { Meta, StoryObj } from '@storybook/react'
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
    onClick: action('onClick'),
  },
}

export const ModelSelector: Story = {
  args: {
    icon: Bot,
    label: 'Claude Opus 4',
    onClick: action('onClick'),
  },
}

export const RepositorySelector: Story = {
  args: {
    icon: Database,
    label: 'my-project',
    onClick: action('onClick'),
  },
}

export const Disabled: Story = {
  args: {
    icon: Brain,
    label: 'Disabled',
    disabled: true,
    onClick: action('onClick'),
  },
}

export const LongLabel: Story = {
  args: {
    icon: Database,
    label: 'very-long-repository-name-that-should-truncate',
    onClick: action('onClick'),
  },
}

export const MultipleSelectors: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <SelectorButton icon={Brain} label="Sonnet 4" onClick={action('model-click')} />
      <SelectorButton icon={Sparkles} label="Edit Mode" onClick={action('mode-click')} />
      <SelectorButton icon={Database} label="my-project" onClick={action('repo-click')} />
    </div>
  ),
}
