import { action } from '@storybook/addon-actions'
import type { Meta, StoryObj } from '@storybook/react'
import { Bot, Database, FileText, GitBranch, Wand2 } from 'lucide-react'
import { useState } from 'react'
import {
  BranchPopoverContent,
  SearchablePopoverContent,
  SimpleListPopoverContent,
} from '../popover-contents'
import { PopoverSelector } from '../popover-selector'

const meta = {
  title: 'Chat/Selectors/PopoverSelector',
  component: PopoverSelector,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="w-[600px] p-8 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PopoverSelector>

export default meta
type Story = StoryObj<typeof meta>

const mockModels = [
  { id: 'opus-4', label: 'Claude Opus 4' },
  { id: 'sonnet-4', label: 'Claude Sonnet 4' },
]

const mockRepositories = [
  { id: 'repo-1', label: 'my-project', description: 'Main application repository' },
  { id: 'repo-2', label: 'backend-api', description: 'REST API server' },
  { id: 'repo-3', label: 'frontend-app', description: 'React frontend application' },
  { id: 'repo-4', label: 'mobile-app', description: 'React Native mobile app' },
  { id: 'repo-5', label: 'shared-lib', description: 'Shared utilities library' },
  { id: 'repo-6', label: 'documentation', description: 'Project documentation' },
  { id: 'repo-7', label: 'infrastructure', description: 'DevOps and infrastructure code' },
  { id: 'repo-8', label: 'microservice-auth', description: 'Authentication microservice' },
]

const mockBranches = [
  { id: 'main', label: 'main' },
  { id: 'develop', label: 'develop' },
  { id: 'feature/authentication', label: 'feature/authentication' },
  { id: 'feature/dashboard', label: 'feature/dashboard' },
  { id: 'bugfix/login-error', label: 'bugfix/login-error' },
  { id: 'bugfix/api-timeout', label: 'bugfix/api-timeout' },
  { id: 'release/v2.0', label: 'release/v2.0' },
]

const mockAgentModes = [
  { id: 'edit', label: 'Edit Mode' },
  { id: 'plan', label: 'Plan Mode' },
]

const mockTemplates = [
  { id: 'template-1', label: 'React Component', description: 'Create a new React component' },
  { id: 'template-2', label: 'Node.js Service', description: 'Express API service template' },
  { id: 'template-3', label: 'Python Script', description: 'Python utility script' },
  { id: 'template-4', label: 'API Endpoint', description: 'REST API endpoint template' },
  { id: 'template-5', label: 'Database Migration', description: 'SQL migration template' },
]

export const Default: Story = {
  render: () => {
    const [value, setValue] = useState('')
    return (
      <PopoverSelector
        icon={Bot}
        value={value}
        displayValue="Select an option"
        tooltip="Choose option"
      >
        <SimpleListPopoverContent
          items={mockModels}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
        />
      </PopoverSelector>
    )
  },
}

export const ModelSelector: Story = {
  render: () => {
    const [value, setValue] = useState('sonnet-4')
    const selectedModel = mockModels.find((m) => m.id === value)
    return (
      <PopoverSelector
        icon={Bot}
        value={value}
        displayValue={selectedModel?.label}
        tooltip="Choose AI model"
      >
        <SimpleListPopoverContent
          items={mockModels}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
        />
      </PopoverSelector>
    )
  },
}

export const RepositorySelector: Story = {
  render: () => {
    const [value, setValue] = useState('repo-1')
    const selectedRepo = mockRepositories.find((r) => r.id === value)
    return (
      <PopoverSelector
        icon={Database}
        value={value}
        displayValue={selectedRepo?.label}
        tooltip="Choose repository"
      >
        <SearchablePopoverContent
          items={mockRepositories}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          placeholder="Search repositories..."
          emptyText="No repositories found"
        />
      </PopoverSelector>
    )
  },
}

export const BranchSelector: Story = {
  render: () => {
    const [value, setValue] = useState('main')
    const selectedBranch = mockBranches.find((b) => b.id === value)
    return (
      <PopoverSelector
        icon={GitBranch}
        value={value}
        displayValue={selectedBranch?.label}
        tooltip="Choose branch"
      >
        <BranchPopoverContent
          items={mockBranches}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          currentBranch="main"
          enableSearch={true}
          placeholder="Search branches..."
        />
      </PopoverSelector>
    )
  },
}

export const AgentModeSelector: Story = {
  render: () => {
    const [value, setValue] = useState('edit')
    const selectedMode = mockAgentModes.find((m) => m.id === value)
    return (
      <PopoverSelector
        icon={Wand2}
        value={value}
        displayValue={selectedMode?.label}
        tooltip="Choose agent mode"
      >
        <SimpleListPopoverContent
          items={mockAgentModes}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
        />
      </PopoverSelector>
    )
  },
}

export const TemplateSelector: Story = {
  render: () => {
    const [value, setValue] = useState('template-1')
    const selectedTemplate = mockTemplates.find((t) => t.id === value)
    return (
      <PopoverSelector
        icon={FileText}
        value={value}
        displayValue={selectedTemplate?.label}
        tooltip="Choose template"
      >
        <SearchablePopoverContent
          items={mockTemplates}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          placeholder="Search templates..."
          emptyText="No templates found"
        />
      </PopoverSelector>
    )
  },
}

export const NoValue: Story = {
  render: () => {
    const [value, setValue] = useState('')
    return (
      <PopoverSelector
        icon={Database}
        value={value}
        placeholder="Select repository"
        tooltip="Choose repository"
      >
        <SearchablePopoverContent
          items={mockRepositories}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          placeholder="Search repositories..."
          emptyText="No repositories found"
        />
      </PopoverSelector>
    )
  },
}

export const Loading: Story = {
  args: {
    icon: Database,
    loading: true,
    placeholder: 'Loading...',
    tooltip: 'Loading repositories',
    children: <div className="p-4 text-sm text-muted-foreground">Loading...</div>,
  },
}

export const Disabled: Story = {
  args: {
    icon: GitBranch,
    disabled: true,
    placeholder: 'Select branch',
    tooltip: 'Select a repository first',
    children: <div className="p-4 text-sm text-muted-foreground">Select a repository first</div>,
  },
}

export const EmptyList: Story = {
  render: () => {
    const [value, setValue] = useState('')
    return (
      <PopoverSelector
        icon={Database}
        value={value}
        placeholder="No repositories"
        tooltip="No repositories available"
      >
        <SearchablePopoverContent
          items={[]}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          placeholder="Search repositories..."
          emptyText="No repositories available"
        />
      </PopoverSelector>
    )
  },
}

export const LongLabels: Story = {
  render: () => {
    const [value, setValue] = useState('long-1')
    const longRepos = [
      {
        id: 'long-1',
        label: 'very-long-repository-name-that-should-truncate-properly',
        description: 'This is a very long description that should also truncate when displayed',
      },
      {
        id: 'long-2',
        label: 'another-extremely-long-repository-name-for-testing',
        description: 'Another long description to test truncation behavior in the list',
      },
    ]
    const selected = longRepos.find((r) => r.id === value)
    return (
      <PopoverSelector
        icon={Database}
        value={value}
        displayValue={selected?.label}
        tooltip="Choose repository"
      >
        <SearchablePopoverContent
          items={longRepos}
          value={value}
          onChange={(newValue) => {
            setValue(newValue)
            action('onChange')(newValue)
          }}
          placeholder="Search..."
        />
      </PopoverSelector>
    )
  },
}

export const MultipleSelectors: Story = {
  render: () => {
    const [model, setModel] = useState('sonnet-4')
    const [mode, setMode] = useState('edit')
    const [repo, setRepo] = useState('repo-1')

    const selectedModel = mockModels.find((m) => m.id === model)
    const selectedMode = mockAgentModes.find((m) => m.id === mode)
    const selectedRepo = mockRepositories.find((r) => r.id === repo)

    return (
      <div className="flex items-center gap-2">
        <PopoverSelector
          icon={Database}
          value={repo}
          displayValue={selectedRepo?.label}
          tooltip="Choose repository"
        >
          <SearchablePopoverContent
            items={mockRepositories}
            value={repo}
            onChange={(newValue) => {
              setRepo(newValue)
              action('repo-change')(newValue)
            }}
            placeholder="Search repositories..."
          />
        </PopoverSelector>

        <PopoverSelector
          icon={Bot}
          value={model}
          displayValue={selectedModel?.label}
          tooltip="Choose model"
        >
          <SimpleListPopoverContent
            items={mockModels}
            value={model}
            onChange={(newValue) => {
              setModel(newValue)
              action('model-change')(newValue)
            }}
          />
        </PopoverSelector>

        <PopoverSelector
          icon={Wand2}
          value={mode}
          displayValue={selectedMode?.label}
          tooltip="Choose mode"
        >
          <SimpleListPopoverContent
            items={mockAgentModes}
            value={mode}
            onChange={(newValue) => {
              setMode(newValue)
              action('mode-change')(newValue)
            }}
          />
        </PopoverSelector>
      </div>
    )
  },
}
