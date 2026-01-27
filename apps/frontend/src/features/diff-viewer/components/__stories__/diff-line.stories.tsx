import { DiffGutter, DiffLine, type DiffLineData, DiffLines } from '@moatless/diff-viewer'
import type { Meta, StoryObj } from '@storybook/react-vite'

// Sample lines for stories
const normalLine: DiffLineData = {
  type: 'normal',
  content: "import React from 'react'",
  oldNumber: 1,
  newNumber: 1,
}

const addedLine: DiffLineData = {
  type: 'add',
  content: "import { cn } from '@/lib/utils'",
  newNumber: 2,
}

const deletedLine: DiffLineData = {
  type: 'del',
  content: "import { classNames } from './utils'",
  oldNumber: 2,
}

const hunkHeader: DiffLineData = {
  type: 'hunk-header',
  content: ' function Button({ children, variant })',
}

const addedLineWithWordChanges: DiffLineData = {
  type: 'add',
  content: "  variant?: 'primary' | 'secondary' | 'ghost'",
  newNumber: 6,
  changes: [
    { type: 'equal', value: "  variant?: 'primary' | 'secondary'" },
    { type: 'add', value: " | 'ghost'" },
  ],
}

const deletedLineWithWordChanges: DiffLineData = {
  type: 'del',
  content: "  variant?: 'primary' | 'secondary'",
  oldNumber: 5,
  changes: [{ type: 'equal', value: "  variant?: 'primary' | 'secondary'" }],
}

// ============ DiffGutter Stories ============
const gutterMeta = {
  title: 'DiffViewer/DiffGutter',
  component: DiffGutter,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="bg-background border rounded">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffGutter>

export default gutterMeta
type GutterStory = StoryObj<typeof gutterMeta>

export const NormalLine: GutterStory = {
  args: {
    oldNumber: 42,
    newNumber: 45,
    type: 'normal',
  },
}

export const AddedLine: GutterStory = {
  args: {
    newNumber: 15,
    type: 'add',
  },
}

export const DeletedLine: GutterStory = {
  args: {
    oldNumber: 23,
    type: 'del',
  },
}

export const HunkHeaderGutter: GutterStory = {
  args: {
    type: 'hunk-header',
  },
}

// ============ DiffLine Stories ============
const lineMeta = {
  title: 'DiffViewer/DiffLine',
  component: DiffLine,
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
} satisfies Meta<typeof DiffLine>

export const Normal: StoryObj<typeof lineMeta> = {
  args: {
    line: normalLine,
  },
}

export const Added: StoryObj<typeof lineMeta> = {
  args: {
    line: addedLine,
  },
}

export const Deleted: StoryObj<typeof lineMeta> = {
  args: {
    line: deletedLine,
  },
}

export const HunkHeader: StoryObj<typeof lineMeta> = {
  args: {
    line: hunkHeader,
  },
}

export const AddedWithWordHighlight: StoryObj<typeof lineMeta> = {
  args: {
    line: addedLineWithWordChanges,
  },
}

export const DeletedWithWordHighlight: StoryObj<typeof lineMeta> = {
  args: {
    line: deletedLineWithWordChanges,
  },
}

// ============ DiffLines Stories ============
const linesMeta = {
  title: 'DiffViewer/DiffLines',
  component: DiffLines,
  parameters: {
    layout: 'padded',
  },
  decorators: [
    (Story) => (
      <div className="bg-background border rounded min-w-[700px]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DiffLines>

export const MultipleLines: StoryObj<typeof linesMeta> = {
  args: {
    lines: [
      hunkHeader,
      normalLine,
      deletedLine,
      addedLine,
      { type: 'normal', content: '', oldNumber: 3, newNumber: 3 },
      deletedLineWithWordChanges,
      addedLineWithWordChanges,
    ],
  },
}

export const EmptyLines: StoryObj<typeof linesMeta> = {
  args: {
    lines: [],
  },
}
