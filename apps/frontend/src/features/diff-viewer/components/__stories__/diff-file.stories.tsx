import {
  type DiffFile,
  DiffFileCard,
  DiffFileHeader,
  type DiffLineData,
  type HiddenChunkData,
} from '@moatless/diff-viewer'
import type { Meta, StoryObj } from '@storybook/react-vite'

// Sample lines for the file
const sampleLines: DiffLineData[] = [
  { type: 'hunk-header', content: ' interface ButtonProps' },
  { type: 'normal', content: '  children: React.ReactNode', oldNumber: 4, newNumber: 4 },
  {
    type: 'del',
    content: "  variant?: 'primary' | 'secondary'",
    oldNumber: 5,
    changes: [{ type: 'equal', value: "  variant?: 'primary' | 'secondary'" }],
  },
  {
    type: 'add',
    content: "  variant?: 'primary' | 'secondary' | 'ghost'",
    newNumber: 5,
    changes: [
      { type: 'equal', value: "  variant?: 'primary' | 'secondary'" },
      { type: 'add', value: " | 'ghost'" },
    ],
  },
  { type: 'add', content: "  size?: 'sm' | 'md' | 'lg'", newNumber: 6 },
  { type: 'normal', content: '  onClick?: () => void', oldNumber: 6, newNumber: 7 },
]

// Sample files
const modifiedFile: DiffFile = {
  filename: 'src/components/button.tsx',
  status: 'modified',
  additions: 5,
  deletions: 3,
  language: 'tsx',
  hunks: [
    {
      oldStart: 1,
      oldLines: 10,
      newStart: 1,
      newLines: 12,
      header: '@@ -1,10 +1,12 @@',
      lines: sampleLines,
    },
  ],
}

const addedFile: DiffFile = {
  filename: 'src/utils/helpers.ts',
  status: 'added',
  additions: 15,
  deletions: 0,
  language: 'typescript',
  hunks: [
    {
      oldStart: 0,
      oldLines: 0,
      newStart: 1,
      newLines: 15,
      header: '@@ -0,0 +1,15 @@',
      lines: [
        { type: 'hunk-header', content: '' },
        { type: 'add', content: '/**', newNumber: 1 },
        { type: 'add', content: ' * Utility helper functions', newNumber: 2 },
        { type: 'add', content: ' */', newNumber: 3 },
        { type: 'add', content: '', newNumber: 4 },
        { type: 'add', content: 'export function formatDate(date: Date): string {', newNumber: 5 },
        { type: 'add', content: "  return date.toLocaleDateString('en-US')", newNumber: 6 },
        { type: 'add', content: '}', newNumber: 7 },
      ],
    },
  ],
}

const deletedFile: DiffFile = {
  filename: 'src/config/old-config.ts',
  status: 'deleted',
  additions: 0,
  deletions: 8,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 8,
      newStart: 0,
      newLines: 0,
      header: '@@ -1,8 +0,0 @@',
      lines: [
        { type: 'hunk-header', content: '' },
        { type: 'del', content: '// Deprecated configuration file', oldNumber: 1 },
        { type: 'del', content: 'export const config = {', oldNumber: 2 },
        { type: 'del', content: "  apiUrl: 'http://localhost:3000',", oldNumber: 3 },
        { type: 'del', content: '  timeout: 5000,', oldNumber: 4 },
        { type: 'del', content: '}', oldNumber: 5 },
      ],
    },
  ],
}

const renamedFile: DiffFile = {
  filename: 'src/utils/string-helpers.ts',
  oldFilename: 'src/utils/helpers.ts',
  status: 'renamed',
  additions: 2,
  deletions: 1,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 5,
      newStart: 1,
      newLines: 6,
      header: '@@ -1,5 +1,6 @@',
      lines: [
        { type: 'hunk-header', content: '' },
        { type: 'normal', content: '/**', oldNumber: 1, newNumber: 1 },
        { type: 'del', content: ' * Helper functions', oldNumber: 2 },
        { type: 'add', content: ' * String helper functions', newNumber: 2 },
        { type: 'add', content: ' * @module string-helpers', newNumber: 3 },
        { type: 'normal', content: ' */', oldNumber: 3, newNumber: 4 },
      ],
    },
  ],
}

// ============ DiffFileHeader Stories ============
const headerMeta = {
  title: 'DiffViewer/DiffFileHeader',
  component: DiffFileHeader,
  parameters: {
    layout: 'padded',
  },
  decorators: [
    (Story) => (
      <div className="bg-background border rounded min-w-[600px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffFileHeader>

export default headerMeta
type HeaderStory = StoryObj<typeof headerMeta>

export const ModifiedFileHeader: HeaderStory = {
  args: {
    file: modifiedFile,
    collapsed: false,
  },
}

export const AddedFileHeader: HeaderStory = {
  args: {
    file: addedFile,
    collapsed: false,
  },
}

export const DeletedFileHeader: HeaderStory = {
  args: {
    file: deletedFile,
    collapsed: false,
  },
}

export const RenamedFileHeader: HeaderStory = {
  args: {
    file: renamedFile,
    collapsed: false,
  },
}

export const CollapsedHeader: HeaderStory = {
  args: {
    file: modifiedFile,
    collapsed: true,
  },
}

export const DeepPathHeader: HeaderStory = {
  args: {
    file: {
      ...modifiedFile,
      filename: 'packages/frontend/src/features/tasks/components/task-header.tsx',
    },
    collapsed: false,
  },
}

// ============ DiffFileCard Stories ============
const cardMeta = {
  title: 'DiffViewer/DiffFileCard',
  component: DiffFileCard,
  parameters: {
    layout: 'padded',
  },
  decorators: [
    (Story) => (
      <div className="bg-background p-4 min-w-[700px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffFileCard>

export const ModifiedFileCard: StoryObj<typeof cardMeta> = {
  args: {
    file: modifiedFile,
  },
}

export const AddedFileCard: StoryObj<typeof cardMeta> = {
  args: {
    file: addedFile,
  },
}

export const DeletedFileCard: StoryObj<typeof cardMeta> = {
  args: {
    file: deletedFile,
  },
}

export const RenamedFileCard: StoryObj<typeof cardMeta> = {
  args: {
    file: renamedFile,
  },
}

export const CollapsedFileCard: StoryObj<typeof cardMeta> = {
  args: {
    file: modifiedFile,
    collapsed: true,
  },
}

// File with hidden chunks (collapsed unchanged lines)
// Using 50 lines to demonstrate partial expansion (shows "All N lines" button when > 40 lines)
const hiddenChunkData: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 50,
  oldStart: 10,
  newStart: 10,
  lines: Array.from({ length: 50 }, (_, i) => ({
    type: 'normal' as const,
    content: `  // Line ${i + 10} of unchanged code - this is a placeholder for collapsed content`,
    oldNumber: i + 10,
    newNumber: i + 10,
  })),
}

const fileWithHiddenChunks: DiffFile = {
  filename: 'src/services/api-client.ts',
  status: 'modified',
  additions: 2,
  deletions: 1,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 30,
      newStart: 1,
      newLines: 31,
      header: '@@ -1,30 +1,31 @@',
      lines: [
        { type: 'hunk-header', content: ' class ApiClient' },
        { type: 'normal', content: 'import axios from "axios"', oldNumber: 1, newNumber: 1 },
        { type: 'normal', content: '', oldNumber: 2, newNumber: 2 },
        { type: 'normal', content: 'export class ApiClient {', oldNumber: 3, newNumber: 3 },
        { type: 'del', content: '  private baseUrl: string', oldNumber: 4 },
        { type: 'add', content: '  private readonly baseUrl: string', newNumber: 4 },
        { type: 'add', content: '  private readonly timeout: number', newNumber: 5 },
        { type: 'normal', content: '', oldNumber: 5, newNumber: 6 },
        { type: 'normal', content: '  constructor(baseUrl: string) {', oldNumber: 6, newNumber: 7 },
        { type: 'normal', content: '    this.baseUrl = baseUrl', oldNumber: 7, newNumber: 8 },
        // Hidden chunk - 15 collapsed unchanged lines
        hiddenChunkData,
        { type: 'normal', content: '  }', oldNumber: 25, newNumber: 26 },
        { type: 'normal', content: '}', oldNumber: 26, newNumber: 27 },
      ],
    },
  ],
}

export const FileWithHiddenChunksMiddle: StoryObj<typeof cardMeta> = {
  args: {
    file: fileWithHiddenChunks,
  },
}

// Hidden chunk at START of file (should show expand DOWN button)
const hiddenChunkAtStart: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 30,
  oldStart: 1,
  newStart: 1,
  lines: Array.from({ length: 30 }, (_, i) => ({
    type: 'normal' as const,
    content: `// Line ${i + 1} - unchanged header comments and imports`,
    oldNumber: i + 1,
    newNumber: i + 1,
  })),
}

const fileWithHiddenChunkAtStart: DiffFile = {
  filename: 'src/utils/helpers.ts',
  status: 'modified',
  additions: 3,
  deletions: 1,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 40,
      newStart: 1,
      newLines: 42,
      header: '@@ -1,40 +1,42 @@',
      lines: [
        { type: 'hunk-header', content: '' },
        // Hidden chunk at START (right after hunk-header)
        hiddenChunkAtStart,
        { type: 'del', content: 'export function oldHelper() {', oldNumber: 31 },
        { type: 'add', content: 'export function newHelper() {', newNumber: 31 },
        { type: 'add', content: '  // Added new functionality', newNumber: 32 },
        { type: 'add', content: '  return true', newNumber: 33 },
        { type: 'normal', content: '}', oldNumber: 32, newNumber: 34 },
      ],
    },
  ],
}

export const FileWithHiddenChunkAtStart: StoryObj<typeof cardMeta> = {
  args: {
    file: fileWithHiddenChunkAtStart,
  },
}

// Hidden chunk at END of file (should show expand UP button)
const hiddenChunkAtEnd: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 25,
  oldStart: 20,
  newStart: 22,
  lines: Array.from({ length: 25 }, (_, i) => ({
    type: 'normal' as const,
    content: `// Line ${i + 20} - unchanged footer code`,
    oldNumber: i + 20,
    newNumber: i + 22,
  })),
}

const fileWithHiddenChunkAtEnd: DiffFile = {
  filename: 'src/config/settings.ts',
  status: 'modified',
  additions: 2,
  deletions: 1,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 45,
      newStart: 1,
      newLines: 47,
      header: '@@ -1,45 +1,47 @@',
      lines: [
        { type: 'hunk-header', content: ' Config settings' },
        { type: 'normal', content: 'export const config = {', oldNumber: 1, newNumber: 1 },
        { type: 'del', content: '  timeout: 5000,', oldNumber: 2 },
        { type: 'add', content: '  timeout: 10000,', newNumber: 2 },
        { type: 'add', content: '  retries: 3,', newNumber: 3 },
        {
          type: 'normal',
          content: '  baseUrl: "https://api.example.com",',
          oldNumber: 3,
          newNumber: 4,
        },
        // Hidden chunk at END (last item)
        hiddenChunkAtEnd,
      ],
    },
  ],
}

export const FileWithHiddenChunkAtEnd: StoryObj<typeof cardMeta> = {
  args: {
    file: fileWithHiddenChunkAtEnd,
  },
}

// File with hidden chunks at ALL positions (start, middle, end)
const hiddenChunkStart: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 15,
  oldStart: 1,
  newStart: 1,
  lines: Array.from({ length: 15 }, (_, i) => ({
    type: 'normal' as const,
    content: `// Header line ${i + 1}`,
    oldNumber: i + 1,
    newNumber: i + 1,
  })),
}

const hiddenChunkMiddle: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 20,
  oldStart: 25,
  newStart: 27,
  lines: Array.from({ length: 20 }, (_, i) => ({
    type: 'normal' as const,
    content: `  // Middle line ${i + 25}`,
    oldNumber: i + 25,
    newNumber: i + 27,
  })),
}

const hiddenChunkEnd: HiddenChunkData = {
  type: 'hidden-chunk',
  lineCount: 18,
  oldStart: 55,
  newStart: 59,
  lines: Array.from({ length: 18 }, (_, i) => ({
    type: 'normal' as const,
    content: `// Footer line ${i + 55}`,
    oldNumber: i + 55,
    newNumber: i + 59,
  })),
}

const fileWithAllPositions: DiffFile = {
  filename: 'src/services/full-example.ts',
  status: 'modified',
  additions: 5,
  deletions: 2,
  language: 'typescript',
  hunks: [
    {
      oldStart: 1,
      oldLines: 73,
      newStart: 1,
      newLines: 77,
      header: '@@ -1,73 +1,77 @@',
      lines: [
        { type: 'hunk-header', content: ' Full example file' },
        // START - should show expand DOWN (↓)
        hiddenChunkStart,
        { type: 'del', content: 'const OLD_CONSTANT = 1', oldNumber: 16 },
        { type: 'add', content: 'const NEW_CONSTANT = 2', newNumber: 16 },
        { type: 'add', content: 'const ANOTHER_CONSTANT = 3', newNumber: 17 },
        { type: 'normal', content: '', oldNumber: 17, newNumber: 18 },
        { type: 'normal', content: 'export class Example {', oldNumber: 18, newNumber: 19 },
        // MIDDLE - should show expand ALL (↕)
        hiddenChunkMiddle,
        { type: 'del', content: '  oldMethod() {}', oldNumber: 45 },
        { type: 'add', content: '  newMethod() {', newNumber: 47 },
        { type: 'add', content: '    return true', newNumber: 48 },
        { type: 'add', content: '  }', newNumber: 49 },
        { type: 'normal', content: '}', oldNumber: 46, newNumber: 50 },
        // END - should show expand UP (↑)
        hiddenChunkEnd,
      ],
    },
  ],
}

export const FileWithAllPositions: StoryObj<typeof cardMeta> = {
  args: {
    file: fileWithAllPositions,
  },
}
