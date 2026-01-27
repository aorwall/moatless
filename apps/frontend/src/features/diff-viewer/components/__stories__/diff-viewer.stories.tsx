import { type DiffFile, type DiffLineData, FileChangeStats, parseDiff } from '@moatless/diff-viewer'
import type { Meta, StoryObj } from '@storybook/react-vite'

// Sample unified diff for demonstration
const sampleDiff = `diff --git a/src/components/button.tsx b/src/components/button.tsx
index 1234567..abcdefg 100644
--- a/src/components/button.tsx
+++ b/src/components/button.tsx
@@ -1,10 +1,12 @@
 import React from 'react'
+import { cn } from '@/lib/utils'

 interface ButtonProps {
   children: React.ReactNode
-  variant?: 'primary' | 'secondary'
+  variant?: 'primary' | 'secondary' | 'ghost'
+  size?: 'sm' | 'md' | 'lg'
   onClick?: () => void
 }

-export function Button({ children, variant = 'primary', onClick }: ButtonProps) {
+export function Button({ children, variant = 'primary', size = 'md', onClick }: ButtonProps) {
   return (
     <button
-      className={\`btn btn-\${variant}\`}
+      className={cn('btn', \`btn-\${variant}\`, \`btn-\${size}\`)}
       onClick={onClick}
     >
       {children}
diff --git a/src/utils/helpers.ts b/src/utils/helpers.ts
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/src/utils/helpers.ts
@@ -0,0 +1,15 @@
+/**
+ * Utility helper functions
+ */
+
+export function formatDate(date: Date): string {
+  return date.toLocaleDateString('en-US', {
+    year: 'numeric',
+    month: 'short',
+    day: 'numeric',
+  })
+}
+
+export function capitalize(str: string): string {
+  return str.charAt(0).toUpperCase() + str.slice(1)
+}
diff --git a/src/config/old-config.ts b/src/config/old-config.ts
deleted file mode 100644
index abcdefg..0000000
--- a/src/config/old-config.ts
+++ /dev/null
@@ -1,8 +0,0 @@
-// Deprecated configuration file
-export const config = {
-  apiUrl: 'http://localhost:3000',
-  timeout: 5000,
-  retries: 3,
-}
-
-export default config
`

// Parse the sample diff
const parsedDiff = parseDiff(sampleDiff)

// Sample files for sidebar stories
const sampleFiles: DiffFile[] = parsedDiff.files

// Sample single file for detailed component stories
const _sampleModifiedFile: DiffFile = sampleFiles[0] ?? {
  filename: 'src/components/button.tsx',
  status: 'modified',
  additions: 5,
  deletions: 3,
  hunks: [],
}

const _sampleAddedFile: DiffFile = sampleFiles[1] ?? {
  filename: 'src/utils/helpers.ts',
  status: 'added',
  additions: 15,
  deletions: 0,
  hunks: [],
}

const _sampleDeletedFile: DiffFile = sampleFiles[2] ?? {
  filename: 'src/config/old-config.ts',
  status: 'deleted',
  additions: 0,
  deletions: 8,
  hunks: [],
}

// Sample lines for line-level stories
const _sampleLines: DiffLineData[] = [
  { type: 'hunk-header', content: ' function formatDate(date: Date)' },
  { type: 'normal', content: "import React from 'react'", oldNumber: 1, newNumber: 1 },
  { type: 'add', content: "import { cn } from '@/lib/utils'", newNumber: 2 },
  { type: 'normal', content: '', oldNumber: 2, newNumber: 3 },
  { type: 'del', content: "  variant?: 'primary' | 'secondary'", oldNumber: 5 },
  {
    type: 'add',
    content: "  variant?: 'primary' | 'secondary' | 'ghost'",
    newNumber: 6,
    changes: [
      { type: 'equal', value: "  variant?: 'primary' | 'secondary'" },
      { type: 'add', value: " | 'ghost'" },
    ],
  },
]

// ============ FileChangeStats Stories ============
const fileChangeStatsMeta = {
  title: 'DiffViewer/FileChangeStats',
  component: FileChangeStats,
  parameters: {
    layout: 'centered',
  },
  decorators: [
    (Story) => (
      <div className="p-4 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FileChangeStats>

export default fileChangeStatsMeta
type FileChangeStatsStory = StoryObj<typeof fileChangeStatsMeta>

export const AdditionsAndDeletions: FileChangeStatsStory = {
  args: {
    additions: 25,
    deletions: 10,
  },
}

export const OnlyAdditions: FileChangeStatsStory = {
  args: {
    additions: 15,
    deletions: 0,
  },
}

export const OnlyDeletions: FileChangeStatsStory = {
  args: {
    additions: 0,
    deletions: 8,
  },
}

export const SmallSize: FileChangeStatsStory = {
  args: {
    additions: 5,
    deletions: 3,
    size: 'sm',
  },
}

export const WithZeros: FileChangeStatsStory = {
  args: {
    additions: 0,
    deletions: 0,
    showZeros: true,
  },
}
