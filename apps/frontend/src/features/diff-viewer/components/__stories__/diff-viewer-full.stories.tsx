import { type DiffFile, parseDiff } from '@moatless/diff-viewer'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { useDiffState } from '../../hooks/use-diff-state'
import { DiffContent } from '../diff-content'
import { DiffSidebar } from '../diff-sidebar'

// Sample diff with many unchanged lines to demonstrate hidden chunks
const diffWithHiddenChunks = `diff --git a/src/services/api-client.ts b/src/services/api-client.ts
index 1234567..abcdefg 100644
--- a/src/services/api-client.ts
+++ b/src/services/api-client.ts
@@ -1,50 +1,52 @@
 import axios from 'axios'
 import { config } from '../config'

 /**
  * API Client for making HTTP requests
  * Handles authentication, error handling, and request/response transformations
  */
 export class ApiClient {
   private baseUrl: string
   private timeout: number
+  private retryCount: number

   constructor(baseUrl: string, timeout = 5000) {
     this.baseUrl = baseUrl
     this.timeout = timeout
+    this.retryCount = 3
   }

   /**
    * Make a GET request
    */
   async get<T>(path: string): Promise<T> {
     const response = await axios.get(\`\${this.baseUrl}\${path}\`, {
       timeout: this.timeout,
     })
     return response.data
   }

   /**
    * Make a POST request
    */
   async post<T>(path: string, data: unknown): Promise<T> {
     const response = await axios.post(\`\${this.baseUrl}\${path}\`, data, {
       timeout: this.timeout,
     })
     return response.data
   }

   /**
    * Make a PUT request
    */
   async put<T>(path: string, data: unknown): Promise<T> {
     const response = await axios.put(\`\${this.baseUrl}\${path}\`, data, {
       timeout: this.timeout,
     })
     return response.data
   }
 }

 // Default instance
-export const apiClient = new ApiClient(config.apiUrl)
+export const apiClient = new ApiClient(config.apiUrl, config.timeout)
`

// Full sample diff demonstrating all features
const fullSampleDiff = `diff --git a/src/components/button.tsx b/src/components/button.tsx
index 1234567..abcdefg 100644
--- a/src/components/button.tsx
+++ b/src/components/button.tsx
@@ -1,15 +1,22 @@
 import React from 'react'
+import { cn } from '@/lib/utils'
+import { Loader2 } from 'lucide-react'

 interface ButtonProps {
   children: React.ReactNode
-  variant?: 'primary' | 'secondary'
+  variant?: 'primary' | 'secondary' | 'ghost' | 'outline'
+  size?: 'sm' | 'md' | 'lg'
+  disabled?: boolean
+  loading?: boolean
   onClick?: () => void
 }

-export function Button({ children, variant = 'primary', onClick }: ButtonProps) {
+export function Button({
+  children,
+  variant = 'primary',
+  size = 'md',
+  disabled = false,
+  loading = false,
+  onClick,
+}: ButtonProps) {
+  const isDisabled = disabled || loading
+
   return (
     <button
-      className={\`btn btn-\${variant}\`}
+      className={cn('btn', \`btn-\${variant}\`, \`btn-\${size}\`, {
+        'opacity-50 cursor-not-allowed': isDisabled,
+      })}
       onClick={onClick}
+      disabled={isDisabled}
     >
+      {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
       {children}
     </button>
   )
diff --git a/src/utils/helpers.ts b/src/utils/helpers.ts
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/src/utils/helpers.ts
@@ -0,0 +1,25 @@
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
+export function formatTime(date: Date): string {
+  return date.toLocaleTimeString('en-US', {
+    hour: '2-digit',
+    minute: '2-digit',
+  })
+}
+
+export function capitalize(str: string): string {
+  return str.charAt(0).toUpperCase() + str.slice(1)
+}
+
+export function truncate(str: string, length: number): string {
+  return str.length > length ? str.slice(0, length) + '...' : str
+}
diff --git a/src/config/old-config.ts b/src/config/old-config.ts
deleted file mode 100644
index abcdefg..0000000
--- a/src/config/old-config.ts
+++ /dev/null
@@ -1,12 +0,0 @@
-// Deprecated configuration file
-// TODO: Remove after migration complete
-
-export const config = {
-  apiUrl: 'http://localhost:3000',
-  timeout: 5000,
-  retries: 3,
-  debug: false,
-}
-
-// Legacy export
-export default config
diff --git a/src/features/auth/login.tsx b/src/features/auth/login.tsx
index 111111..222222 100644
--- a/src/features/auth/login.tsx
+++ b/src/features/auth/login.tsx
@@ -5,7 +5,7 @@ import { Button } from '@/components/button'
 export function LoginPage() {
   const [email, setEmail] = useState('')
   const [password, setPassword] = useState('')
-  const [error, setError] = useState(null)
+  const [error, setError] = useState<string | null>(null)

   const handleSubmit = async (e: React.FormEvent) => {
     e.preventDefault()
@@ -15,6 +15,10 @@ export function LoginPage() {
       setError(err.message)
     }
   }
+
+  const handleForgotPassword = () => {
+    // TODO: Implement forgot password flow
+  }

   return (
     <form onSubmit={handleSubmit}>
diff --git a/package.json b/package.json
index aaa111..bbb222 100644
--- a/package.json
+++ b/package.json
@@ -12,6 +12,7 @@
   "dependencies": {
     "react": "^19.0.0",
     "react-dom": "^19.0.0",
+    "lucide-react": "^0.400.0",
     "@tanstack/react-query": "^5.0.0"
   }
 }
`

// Parse the diff
const parsedFiles = parseDiff(fullSampleDiff).files

// Interactive full diff viewer component
function FullDiffViewer({ files }: { files: DiffFile[] }) {
  const filenames = files.map((f) => f.filename)
  const {
    state,
    toggleFileCollapsed,
    setSelectedFile,
    setSearchQuery,
    isFileCollapsed,
    collapseAll,
    expandAll,
  } = useDiffState(filenames)

  const handleFileSelect = (filename: string) => {
    setSelectedFile(filename)
    if (isFileCollapsed(filename)) {
      toggleFileCollapsed(filename)
    }
  }

  return (
    <div className="flex h-full bg-background">
      {/* Sidebar */}
      <div className="w-64 shrink-0">
        <DiffSidebar
          files={files}
          selectedFile={state.selectedFile}
          onFileSelect={handleFileSelect}
          searchQuery={state.searchQuery}
          onSearchChange={setSearchQuery}
        />
      </div>

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Toolbar */}
        <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-muted/20">
          <button
            type="button"
            onClick={expandAll}
            className="px-3 py-1 text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-muted rounded"
          >
            Expand all
          </button>
          <button
            type="button"
            onClick={collapseAll}
            className="px-3 py-1 text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-muted rounded"
          >
            Collapse all
          </button>
        </div>

        {/* Diff content */}
        <div className="flex-1 min-h-0 overflow-auto">
          <DiffContent
            files={files}
            selectedFile={state.selectedFile}
            isFileCollapsed={isFileCollapsed}
            onToggleCollapsed={toggleFileCollapsed}
          />
        </div>
      </div>
    </div>
  )
}

// ============ Full DiffViewer Stories ============
const meta = {
  title: 'DiffViewer/FullViewer',
  component: FullDiffViewer,
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <div className="h-screen w-screen">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FullDiffViewer>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  args: {
    files: parsedFiles,
  },
}

export const SingleFile: Story = {
  args: {
    files: parsedFiles.slice(0, 1),
  },
}

export const ManyFiles: Story = {
  args: {
    files: [
      ...parsedFiles,
      {
        filename: 'src/components/card.tsx',
        status: 'modified',
        additions: 8,
        deletions: 2,
        language: 'tsx',
        hunks: parsedFiles[0]?.hunks ?? [],
      },
      {
        filename: 'src/components/input.tsx',
        status: 'added',
        additions: 45,
        deletions: 0,
        language: 'tsx',
        hunks: parsedFiles[1]?.hunks ?? [],
      },
      {
        filename: 'src/styles/globals.css',
        status: 'modified',
        additions: 20,
        deletions: 5,
        language: 'css',
        hunks: [],
      },
      {
        filename: 'README.md',
        status: 'modified',
        additions: 10,
        deletions: 3,
        language: 'markdown',
        hunks: [],
      },
    ],
  },
}

export const NoChanges: Story = {
  args: {
    files: [],
  },
  decorators: [
    (Story) => (
      <div className="h-screen w-screen flex items-center justify-center bg-background">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">No changes</p>
          <p className="text-sm">There are no uncommitted changes to display</p>
        </div>
      </div>
    ),
  ],
}

// Parse the diff with hidden chunks (uses default threshold of 8)
const filesWithHiddenChunks = parseDiff(diffWithHiddenChunks).files

export const WithHiddenChunks: Story = {
  args: {
    files: filesWithHiddenChunks,
  },
}
