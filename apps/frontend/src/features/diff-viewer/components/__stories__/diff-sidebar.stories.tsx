import type { DiffFile } from '@moatless/diff-viewer'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { DiffSidebar } from '../diff-sidebar'
import { DiffSidebarItem } from '../diff-sidebar-item'

// Sample files for sidebar
const sampleFiles: DiffFile[] = [
  {
    filename: 'src/components/button.tsx',
    status: 'modified',
    additions: 5,
    deletions: 3,
    language: 'tsx',
    hunks: [],
  },
  {
    filename: 'src/utils/helpers.ts',
    status: 'added',
    additions: 15,
    deletions: 0,
    language: 'typescript',
    hunks: [],
  },
  {
    filename: 'src/config/old-config.ts',
    status: 'deleted',
    additions: 0,
    deletions: 8,
    language: 'typescript',
    hunks: [],
  },
  {
    filename: 'src/utils/string-helpers.ts',
    oldFilename: 'src/utils/helpers.ts',
    status: 'renamed',
    additions: 2,
    deletions: 1,
    language: 'typescript',
    hunks: [],
  },
  {
    filename: 'src/features/auth/login.tsx',
    status: 'modified',
    additions: 12,
    deletions: 5,
    language: 'tsx',
    hunks: [],
  },
  {
    filename: 'src/features/auth/logout.tsx',
    status: 'modified',
    additions: 3,
    deletions: 2,
    language: 'tsx',
    hunks: [],
  },
  {
    filename: 'package.json',
    status: 'modified',
    additions: 2,
    deletions: 1,
    language: 'json',
    hunks: [],
  },
]

// ============ DiffSidebarItem Stories ============
const itemMeta = {
  title: 'DiffViewer/DiffSidebarItem',
  component: DiffSidebarItem,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="bg-background border rounded w-64">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffSidebarItem>

export default itemMeta
type ItemStory = StoryObj<typeof itemMeta>

export const ModifiedItem: ItemStory = {
  args: {
    file: sampleFiles[0]!,
    isSelected: false,
    onClick: () => undefined,
  },
}

export const AddedItem: ItemStory = {
  args: {
    file: sampleFiles[1]!,
    isSelected: false,
    onClick: () => undefined,
  },
}

export const DeletedItem: ItemStory = {
  args: {
    file: sampleFiles[2]!,
    isSelected: false,
    onClick: () => undefined,
  },
}

export const RenamedItem: ItemStory = {
  args: {
    file: sampleFiles[3]!,
    isSelected: false,
    onClick: () => undefined,
  },
}

export const SelectedItem: ItemStory = {
  args: {
    file: sampleFiles[0]!,
    isSelected: true,
    onClick: () => undefined,
  },
}

export const DeepPathItem: ItemStory = {
  args: {
    file: sampleFiles[4]!,
    isSelected: false,
    onClick: () => undefined,
  },
}

// ============ DiffSidebar Stories ============
const sidebarMeta = {
  title: 'DiffViewer/DiffSidebar',
  component: DiffSidebar,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="bg-background border rounded w-64 h-[500px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffSidebar>

function InteractiveSidebar() {
  const [selectedFile, setSelectedFile] = useState<string | undefined>(undefined)
  const [searchQuery, setSearchQuery] = useState('')

  return (
    <DiffSidebar
      files={sampleFiles}
      selectedFile={selectedFile}
      onFileSelect={setSelectedFile}
      searchQuery={searchQuery}
      onSearchChange={setSearchQuery}
    />
  )
}

export const Default: StoryObj<typeof sidebarMeta> = {
  render: () => <InteractiveSidebar />,
}

export const WithSelection: StoryObj<typeof sidebarMeta> = {
  args: {
    files: sampleFiles,
    selectedFile: 'src/components/button.tsx',
    onFileSelect: () => undefined,
    searchQuery: '',
    onSearchChange: () => undefined,
  },
}

export const WithSearch: StoryObj<typeof sidebarMeta> = {
  args: {
    files: sampleFiles,
    selectedFile: undefined,
    onFileSelect: () => undefined,
    searchQuery: 'auth',
    onSearchChange: () => undefined,
  },
}

export const EmptyState: StoryObj<typeof sidebarMeta> = {
  args: {
    files: [],
    selectedFile: undefined,
    onFileSelect: () => undefined,
    searchQuery: '',
    onSearchChange: () => undefined,
  },
}

export const NoSearchResults: StoryObj<typeof sidebarMeta> = {
  args: {
    files: sampleFiles,
    selectedFile: undefined,
    onFileSelect: () => undefined,
    searchQuery: 'nonexistent',
    onSearchChange: () => undefined,
  },
}
