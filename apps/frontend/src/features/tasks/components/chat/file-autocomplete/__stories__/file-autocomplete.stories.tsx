import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import { FileAutocomplete } from '../file-autocomplete'
import type { FileAutocompleteItem } from '../types'

const meta = {
  title: 'Chat/FileAutocomplete',
  component: FileAutocomplete,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="w-[600px] p-4 bg-background">
        <div className="relative border border-dashed border-muted p-4 rounded-md min-h-[300px]">
          <div className="text-xs text-muted-foreground mb-2">
            Type "@filename" to trigger autocomplete
          </div>
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof FileAutocomplete>

export default meta
type Story = StoryObj<typeof meta>

// Mock data for file search results (name-based path matching)
const mockButtonResults: FileAutocompleteItem[] = [
  {
    path: 'src/components/button.tsx',
    absolutePath: '/project/src/components/button.tsx',
    score: 0.95,
  },
  {
    path: 'src/components/ui/button.tsx',
    absolutePath: '/project/src/components/ui/button.tsx',
    score: 0.9,
  },
  {
    path: 'src/components/icon-button.tsx',
    absolutePath: '/project/src/components/icon-button.tsx',
    score: 0.85,
  },
  {
    path: 'src/stories/button.stories.tsx',
    absolutePath: '/project/src/stories/button.stories.tsx',
    score: 0.82,
  },
  {
    path: 'src/__tests__/button.test.tsx',
    absolutePath: '/project/src/__tests__/button.test.tsx',
    score: 0.8,
  },
]

const mockHookResults: FileAutocompleteItem[] = [
  {
    path: 'src/hooks/use-tasks.ts',
    absolutePath: '/project/src/hooks/use-tasks.ts',
    score: 0.92,
  },
  {
    path: 'src/hooks/use-upload.ts',
    absolutePath: '/project/src/hooks/use-upload.ts',
    score: 0.9,
  },
  {
    path: 'src/hooks/use-repositories.ts',
    absolutePath: '/project/src/hooks/use-repositories.ts',
    score: 0.88,
  },
  {
    path: 'src/lib/hooks/use-theme.ts',
    absolutePath: '/project/src/lib/hooks/use-theme.ts',
    score: 0.85,
  },
]

const mockTaskResults: FileAutocompleteItem[] = [
  {
    path: 'src/features/tasks/task-card.tsx',
    absolutePath: '/project/src/features/tasks/task-card.tsx',
    score: 0.95,
  },
  {
    path: 'src/features/tasks/task-list.tsx',
    absolutePath: '/project/src/features/tasks/task-list.tsx',
    score: 0.93,
  },
  {
    path: 'src/features/tasks/hooks/use-tasks.ts',
    absolutePath: '/project/src/features/tasks/hooks/use-tasks.ts',
    score: 0.9,
  },
  {
    path: 'src/routes/tasks/$task-id/index.tsx',
    absolutePath: '/project/src/routes/tasks/$task-id/index.tsx',
    score: 0.88,
  },
  {
    path: 'src/types/task.ts',
    absolutePath: '/project/src/types/task.ts',
    score: 0.85,
  },
  {
    path: '__tests__/task-service.test.ts',
    absolutePath: '/project/__tests__/task-service.test.ts',
    score: 0.82,
  },
]

const mockManyResults: FileAutocompleteItem[] = [
  {
    path: 'src/components/button.tsx',
    absolutePath: '/project/src/components/button.tsx',
    score: 0.95,
  },
  {
    path: 'src/components/card.tsx',
    absolutePath: '/project/src/components/card.tsx',
    score: 0.94,
  },
  {
    path: 'src/components/dialog.tsx',
    absolutePath: '/project/src/components/dialog.tsx',
    score: 0.93,
  },
  {
    path: 'src/components/dropdown.tsx',
    absolutePath: '/project/src/components/dropdown.tsx',
    score: 0.92,
  },
  {
    path: 'src/components/input.tsx',
    absolutePath: '/project/src/components/input.tsx',
    score: 0.91,
  },
  {
    path: 'src/components/select.tsx',
    absolutePath: '/project/src/components/select.tsx',
    score: 0.9,
  },
  {
    path: 'src/components/textarea.tsx',
    absolutePath: '/project/src/components/textarea.tsx',
    score: 0.89,
  },
  {
    path: 'src/components/tooltip.tsx',
    absolutePath: '/project/src/components/tooltip.tsx',
    score: 0.88,
  },
  {
    path: 'src/components/popover.tsx',
    absolutePath: '/project/src/components/popover.tsx',
    score: 0.87,
  },
  {
    path: 'src/components/tabs.tsx',
    absolutePath: '/project/src/components/tabs.tsx',
    score: 0.86,
  },
  {
    path: 'src/components/accordion.tsx',
    absolutePath: '/project/src/components/accordion.tsx',
    score: 0.85,
  },
  {
    path: 'src/components/alert.tsx',
    absolutePath: '/project/src/components/alert.tsx',
    score: 0.84,
  },
  {
    path: 'src/components/badge.tsx',
    absolutePath: '/project/src/components/badge.tsx',
    score: 0.83,
  },
  {
    path: 'src/components/checkbox.tsx',
    absolutePath: '/project/src/components/checkbox.tsx',
    score: 0.82,
  },
  {
    path: 'src/components/radio.tsx',
    absolutePath: '/project/src/components/radio.tsx',
    score: 0.81,
  },
  {
    path: 'src/components/switch.tsx',
    absolutePath: '/project/src/components/switch.tsx',
    score: 0.8,
  },
  {
    path: 'src/components/slider.tsx',
    absolutePath: '/project/src/components/slider.tsx',
    score: 0.79,
  },
  {
    path: 'src/components/progress.tsx',
    absolutePath: '/project/src/components/progress.tsx',
    score: 0.78,
  },
]

const mockLongPathResults: FileAutocompleteItem[] = [
  {
    path: 'src/features/authentication/components/config/auth-config-provider.tsx',
    absolutePath: '/project/src/features/authentication/components/config/auth-config-provider.tsx',
    score: 0.92,
  },
  {
    path: 'src/features/authentication/components/config/auth-config-form.tsx',
    absolutePath: '/project/src/features/authentication/components/config/auth-config-form.tsx',
    score: 0.9,
  },
  {
    path: 'config/environment/development.config.ts',
    absolutePath: '/project/config/environment/development.config.ts',
    score: 0.88,
  },
]

const mockReadmeResults: FileAutocompleteItem[] = [
  { path: 'README.md', absolutePath: '/project/README.md', score: 0.95 },
  {
    path: 'docs/README.md',
    absolutePath: '/project/docs/README.md',
    score: 0.85,
  },
]

const mockTestResults: FileAutocompleteItem[] = [
  {
    path: 'src/components/button.test.tsx',
    absolutePath: '/project/src/components/button.test.tsx',
    score: 0.95,
  },
  {
    path: 'src/hooks/use-tasks.test.ts',
    absolutePath: '/project/src/hooks/use-tasks.test.ts',
    score: 0.93,
  },
  {
    path: '__tests__/integration/api.test.ts',
    absolutePath: '/project/__tests__/integration/api.test.ts',
    score: 0.88,
  },
  {
    path: 'test/utils/test-helpers.ts',
    absolutePath: '/project/test/utils/test-helpers.ts',
    score: 0.82,
  },
]

export const Loading: Story = {
  args: {
    results: [],
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: true,
    error: null,
    position: 'below',
  },
}

export const WithResults: Story = {
  args: {
    results: mockButtonResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const WithSelectedItem: Story = {
  args: {
    results: mockButtonResults,
    selectedIndex: 2,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const ManyResults: Story = {
  args: {
    results: mockManyResults,
    selectedIndex: 8,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const NoResults: Story = {
  args: {
    results: [],
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const ErrorState: Story = {
  args: {
    results: [],
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: new Error('Failed to search files'),
    position: 'below',
  },
}

export const PositionAbove: Story = {
  args: {
    results: mockTaskResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'above',
  },
}

export const PositionBelow: Story = {
  args: {
    results: mockTaskResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const InteractiveSelection: Story = {
  args: {} as any,
  render: () => {
    const [selectedFile, setSelectedFile] = useState<string | null>(null)

    return (
      <div>
        <FileAutocomplete
          results={mockHookResults}
          selectedIndex={0}
          onSelect={(file) => {
            setSelectedFile(file.path)
            fn()(file)
          }}
          isLoading={false}
          error={null}
          position="below"
        />
        {selectedFile && (
          <div className="mt-4 p-2 bg-accent rounded-md text-sm">
            Selected: <strong>{selectedFile}</strong>
          </div>
        )}
      </div>
    )
  },
}

export const LongFilePaths: Story = {
  args: {
    results: mockLongPathResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const FewResults: Story = {
  args: {
    results: mockReadmeResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}

export const PartialMatch: Story = {
  args: {
    results: mockTestResults,
    selectedIndex: 0,
    onSelect: fn(),
    isLoading: false,
    error: null,
    position: 'below',
  },
}
