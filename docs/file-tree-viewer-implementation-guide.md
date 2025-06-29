# File Tree Viewer Implementation Guide

This guide outlines how to implement a file tree viewer UI for tasks in the Moatless Vibe UI project, following the established architecture patterns.

## Overview

The file tree viewer will allow users to browse and view files within a task's workspace. This is a read-only feature that integrates with the existing task infrastructure.

## Architecture

### Backend Implementation

#### API Routes (`server/routes/tasks.ts`)

The backend routes need to be implemented to support file operations. Here's how to add them:

```typescript
// Add to server/routes/tasks.ts

// List files in a task's workspace
fastify.get<{
  Params: { taskId: string }
  Querystring: { path?: string; recursive?: string }
}>('/tasks/:taskId/files', async (request, reply) => {
  const { taskId } = request.params
  const { path = '/', recursive = 'false' } = request.query
  
  const task = await storageService.getTask(taskId)
  if (!task) {
    return reply.status(404).send({ error: 'Task not found' })
  }

  const workspace = await workspaceService.getWorkspace(taskId)
  if (!workspace) {
    return reply.status(404).send({ error: 'Workspace not found' })
  }

  if (!workspace.canListFiles()) {
    return reply.status(403).send({ error: 'File listing not supported' })
  }

  try {
    const files = await workspace.listFiles(path, recursive === 'true')
    return { files, totalCount: files.length }
  } catch (error) {
    fastify.log.error({ error, taskId, path }, 'Failed to list files')
    return reply.status(500).send({ error: 'Failed to list files' })
  }
})

// Read a specific file
fastify.get<{
  Params: { taskId: string; '*': string }
}>('/tasks/:taskId/files/*', async (request, reply) => {
  const { taskId } = request.params
  const filePath = request.params['*']
  
  if (!filePath) {
    return reply.status(400).send({ error: 'File path is required' })
  }

  const task = await storageService.getTask(taskId)
  if (!task) {
    return reply.status(404).send({ error: 'Task not found' })
  }

  const workspace = await workspaceService.getWorkspace(taskId)
  if (!workspace) {
    return reply.status(404).send({ error: 'Workspace not found' })
  }

  if (!workspace.canReadWriteFiles()) {
    return reply.status(403).send({ error: 'File reading not supported' })
  }

  try {
    const content = await workspace.readFile(filePath)
    return { content, encoding: 'utf-8' }
  } catch (error) {
    if (error.code === 'ENOENT') {
      return reply.status(404).send({ error: 'File not found' })
    }
    fastify.log.error({ error, taskId, filePath }, 'Failed to read file')
    return reply.status(500).send({ error: 'Failed to read file' })
  }
})

// Search files
fastify.post<{
  Params: { taskId: string }
  Body: { query: string }
}>('/tasks/:taskId/search-files', async (request, reply) => {
  const { taskId } = request.params
  const { query } = request.body
  
  if (!query) {
    return reply.status(400).send({ error: 'Search query is required' })
  }

  const task = await storageService.getTask(taskId)
  if (!task) {
    return reply.status(404).send({ error: 'Task not found' })
  }

  const workspace = await workspaceService.getWorkspace(taskId)
  if (!workspace) {
    return reply.status(404).send({ error: 'Workspace not found' })
  }

  if (!workspace.canSearchFiles()) {
    return reply.status(403).send({ error: 'File search not supported' })
  }

  try {
    const results = await workspace.searchFiles(query)
    return { results }
  } catch (error) {
    fastify.log.error({ error, taskId, query }, 'Failed to search files')
    return reply.status(500).send({ error: 'Failed to search files' })
  }
})
```

#### Workspace Service (`server/services/workspace.service.ts`)

The workspace service acts as an abstraction layer that delegates file operations to the appropriate workspace implementation:

```typescript
// Add these methods to WorkspaceService class

async listFiles(taskId: string, path: string, recursive: boolean): Promise<string[]> {
  const workspace = await this.getWorkspace(taskId)
  if (!workspace) {
    throw new Error('Workspace not found')
  }

  if ('listFiles' in workspace && typeof workspace.listFiles === 'function') {
    return workspace.listFiles(path, recursive)
  }

  throw new Error('File listing not supported for this workspace type')
}

async readFile(taskId: string, filePath: string): Promise<string> {
  const workspace = await this.getWorkspace(taskId)
  if (!workspace) {
    throw new Error('Workspace not found')
  }

  if ('readFile' in workspace && typeof workspace.readFile === 'function') {
    return workspace.readFile(filePath)
  }

  throw new Error('File reading not supported for this workspace type')
}

async searchFiles(taskId: string, query: string): Promise<FileSearchResult[]> {
  const workspace = await this.getWorkspace(taskId)
  if (!workspace) {
    throw new Error('Workspace not found')
  }

  if ('searchFiles' in workspace && typeof workspace.searchFiles === 'function') {
    return workspace.searchFiles(query)
  }

  throw new Error('File search not supported for this workspace type')
}

// Helper methods for capability checking
canListFiles(workspace: Workspace): boolean {
  return 'listFiles' in workspace && typeof workspace.listFiles === 'function'
}

canReadWriteFiles(workspace: Workspace): boolean {
  return 'readFile' in workspace && typeof workspace.readFile === 'function'
}

canSearchFiles(workspace: Workspace): boolean {
  return 'searchFiles' in workspace && typeof workspace.searchFiles === 'function'
}
```

#### Local Workspace Implementation (`server/workspaces/local.ts`)

The LocalWorkspace class needs to implement the file operations with proper security:

```typescript
// Add these methods to LocalWorkspace class

async listFiles(relativePath: string = '/', recursive: boolean = false): Promise<string[]> {
  const absolutePath = this.resolvePath(relativePath)
  
  // Security: Ensure path is within workspace
  if (!this.isPathWithinWorkspace(absolutePath)) {
    throw new Error('Access denied: Path outside workspace')
  }

  const files: string[] = []
  
  async function walkDir(dir: string, baseDir: string) {
    const entries = await fs.readdir(dir, { withFileTypes: true })
    
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name)
      const relativePath = path.relative(baseDir, fullPath)
      
      // Skip hidden files and common ignore patterns
      if (entry.name.startsWith('.') || entry.name === 'node_modules') {
        continue
      }
      
      if (entry.isDirectory()) {
        files.push(relativePath + '/')
        if (recursive) {
          await walkDir(fullPath, baseDir)
        }
      } else {
        files.push(relativePath)
      }
    }
  }
  
  await walkDir(absolutePath, this.workspacePath)
  return files.sort()
}

async readFile(relativePath: string): Promise<string> {
  const absolutePath = this.resolvePath(relativePath)
  
  // Security: Ensure path is within workspace
  if (!this.isPathWithinWorkspace(absolutePath)) {
    throw new Error('Access denied: Path outside workspace')
  }

  // Security: Check file exists and is readable
  const stats = await fs.stat(absolutePath)
  if (!stats.isFile()) {
    throw new Error('Path is not a file')
  }

  // Read file with size limit (e.g., 10MB)
  const MAX_FILE_SIZE = 10 * 1024 * 1024
  if (stats.size > MAX_FILE_SIZE) {
    throw new Error('File too large to read')
  }

  return fs.readFile(absolutePath, 'utf-8')
}

async searchFiles(query: string): Promise<FileSearchResult[]> {
  // Use ripgrep or similar for efficient searching
  const { stdout } = await execa('rg', [
    '--json',
    '--max-count', '10', // Limit matches per file
    '--type-add', 'web:*.{html,css,js,jsx,ts,tsx}',
    query,
    this.workspacePath
  ])

  const results: FileSearchResult[] = []
  const lines = stdout.split('\n').filter(Boolean)
  
  for (const line of lines) {
    try {
      const data = JSON.parse(line)
      if (data.type === 'match') {
        const relativePath = path.relative(this.workspacePath, data.data.path.text)
        
        const existing = results.find(r => r.path === relativePath)
        if (existing) {
          existing.matches++
        } else {
          results.push({
            path: relativePath,
            matches: 1
          })
        }
      }
    } catch (e) {
      // Skip invalid JSON lines
    }
  }

  return results
}

// Helper methods for security
private resolvePath(relativePath: string): string {
  // Normalize and resolve the path
  const normalized = path.normalize(relativePath)
  return path.resolve(this.workspacePath, normalized)
}

private isPathWithinWorkspace(absolutePath: string): boolean {
  // Ensure the resolved path is within workspace boundaries
  const relative = path.relative(this.workspacePath, absolutePath)
  return !relative.startsWith('..') && !path.isAbsolute(relative)
}
```

#### Type Definitions

Add these types to `shared/types/workspace.ts`:

```typescript
export interface FileSearchResult {
  path: string
  matches: number
}

export interface WorkspaceCapabilities {
  canListFiles: boolean
  canReadWriteFiles: boolean
  canSearchFiles: boolean
  canExecuteCode: boolean
}
```

### Security Considerations

The backend implementation includes several security measures:

1. **Path Traversal Prevention**:
   - All paths are normalized and resolved relative to workspace root
   - Paths outside workspace boundaries are rejected
   - Symbolic links that point outside workspace are not followed

2. **File Size Limits**:
   - Large files (>10MB) are rejected to prevent memory issues
   - Binary files are detected and rejected

3. **Access Control**:
   - Each workspace type declares its capabilities
   - Operations are only allowed if the workspace supports them
   - Task ownership is verified before any operation

4. **Input Validation**:
   - All user inputs are validated and sanitized
   - File paths are checked for invalid characters
   - Search queries are escaped to prevent injection

5. **Error Handling**:
   - Sensitive error details are logged but not exposed to clients
   - Generic error messages are returned to prevent information leakage

### Frontend Implementation

#### Feature Structure

Create a new feature module for the file viewer:

```
client/src/features/file-viewer/
├── api/
│   ├── use-file-list.ts      # React Query hook for listing files
│   ├── use-file-content.ts   # React Query hook for reading files
│   └── use-file-search.ts    # React Query hook for searching files
├── components/
│   ├── file-tree.tsx         # Main file tree component
│   ├── file-tree-item.tsx    # Individual tree node component
│   ├── file-viewer.tsx       # File content viewer
│   ├── file-search.tsx       # File search component
│   └── file-breadcrumbs.tsx  # Breadcrumb navigation
├── hooks/
│   ├── use-file-tree-state.ts # Tree expansion/selection state
│   └── use-file-navigation.ts # Navigation logic
├── types/
│   └── file-tree.ts          # TypeScript types
└── index.ts                  # Feature exports
```

#### API Hooks Implementation

##### `use-file-list.ts`
```typescript
import { useQuery } from '@tanstack/react-query';
import { apiClient } from '@/lib/api/client';
import { queryKeys } from '@/lib/api/query-keys';

interface FileListParams {
  taskId: string;
  path?: string;
  recursive?: boolean;
}

export function useFileList({ taskId, path = '/', recursive = false }: FileListParams) {
  return useQuery({
    queryKey: queryKeys.taskFileList(taskId, path, recursive),
    queryFn: async () => {
      const params = new URLSearchParams();
      if (path) params.append('path', path);
      if (recursive) params.append('recursive', 'true');
      
      const response = await apiClient.get(`/tasks/${taskId}/files?${params}`);
      return response.data;
    },
    enabled: !!taskId,
  });
}
```

##### `use-file-content.ts`
```typescript
import { useQuery } from '@tanstack/react-query';
import { apiClient } from '@/lib/api/client';
import { queryKeys } from '@/lib/api/query-keys';

export function useFileContent(taskId: string, filePath: string) {
  return useQuery({
    queryKey: queryKeys.taskFileContent(taskId, filePath),
    queryFn: async () => {
      const response = await apiClient.get(`/tasks/${taskId}/files/${filePath}`);
      return response.data;
    },
    enabled: !!taskId && !!filePath,
  });
}
```

#### Component Implementation

##### File Tree Component
```typescript
// file-tree.tsx
import { useState } from 'react';
import { FileTreeItem } from './file-tree-item';
import { useFileList } from '../api/use-file-list';
import { FileTreeSkeleton } from './file-tree-skeleton';
import { ErrorAlert } from '@/lib/components/ui/error-alert';

interface FileTreeProps {
  taskId: string;
  onFileSelect: (filePath: string) => void;
}

export function FileTree({ taskId, onFileSelect }: FileTreeProps) {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const { data, isLoading, error } = useFileList({ taskId, recursive: true });

  if (error) {
    return <ErrorAlert error={error} />;
  }

  if (isLoading) {
    return <FileTreeSkeleton />;
  }

  if (!data?.files || data.files.length === 0) {
    return <div className="p-4 text-muted-foreground">No files found</div>;
  }

  // Build tree structure from flat file list
  const tree = buildTreeStructure(data.files);

  return (
    <div className="file-tree">
      {renderTree(tree, expandedPaths, setExpandedPaths, onFileSelect)}
    </div>
  );
}
```

##### File Viewer Component
```typescript
// file-viewer.tsx
import { useFileContent } from '../api/use-file-content';
import { CodeBlock } from '@/lib/components/ui/code-block';
import { FileViewerSkeleton } from './file-viewer-skeleton';
import { ErrorAlert } from '@/lib/components/ui/error-alert';

interface FileViewerProps {
  taskId: string;
  filePath: string;
}

export function FileViewer({ taskId, filePath }: FileViewerProps) {
  const { data, isLoading, error } = useFileContent(taskId, filePath);

  if (error) {
    return <ErrorAlert error={error} />;
  }

  if (isLoading) {
    return <FileViewerSkeleton />;
  }

  if (!data?.content) {
    return <div className="p-4 text-muted-foreground">File is empty</div>;
  }

  return (
    <div className="file-viewer">
      <div className="file-header p-4 border-b">
        <h3 className="text-sm font-medium">{filePath}</h3>
      </div>
      <div className="file-content">
        <CodeBlock 
          code={data.content} 
          language={getLanguageFromPath(filePath)}
          showLineNumbers
        />
      </div>
    </div>
  );
}
```

#### Integration with Task Context

Add file viewer to the task detail page:

```typescript
// routes/tasks/task-detail.tsx
import { FileTree } from '@/features/file-viewer/components/file-tree';
import { FileViewer } from '@/features/file-viewer/components/file-viewer';
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from '@/lib/components/ui/resizable';

export function TaskDetail() {
  const { taskId } = useParams();
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  return (
    <TaskProvider taskId={taskId}>
      <div className="task-detail">
        {/* Existing task header and content */}
        
        <ResizablePanelGroup direction="horizontal">
          <ResizablePanel defaultSize={25} minSize={15}>
            <FileTree 
              taskId={taskId} 
              onFileSelect={setSelectedFile}
            />
          </ResizablePanel>
          
          <ResizableHandle />
          
          <ResizablePanel defaultSize={75}>
            {selectedFile ? (
              <FileViewer 
                taskId={taskId} 
                filePath={selectedFile}
              />
            ) : (
              <div className="flex items-center justify-center h-full text-muted-foreground">
                Select a file to view
              </div>
            )}
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </TaskProvider>
  );
}
```

### Query Key Management

Add to `client/src/lib/api/query-keys.ts`:

```typescript
export const queryKeys = {
  // ... existing keys
  
  taskFileList: (taskId: string, path?: string, recursive?: boolean) => 
    [...queryKeys.task(taskId), 'files', { path, recursive }] as const,
  
  taskFileContent: (taskId: string, filePath: string) => 
    [...queryKeys.task(taskId), 'file', filePath] as const,
};
```

### State Management

For complex file tree state, create a custom hook:

```typescript
// use-file-tree-state.ts
export function useFileTreeState() {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');

  const toggleExpanded = (path: string) => {
    setExpandedPaths(prev => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  return {
    expandedPaths,
    selectedPath,
    searchQuery,
    toggleExpanded,
    setSelectedPath,
    setSearchQuery,
  };
}
```

### Error Handling

Follow the project's error handling patterns:

1. Use `ErrorBoundary` for component-level error isolation
2. Handle API errors with proper error messages
3. Show loading skeletons during data fetching
4. Display empty states when no data is available

### Performance Considerations

1. **Virtual Scrolling**: For large file trees, implement virtual scrolling
2. **Lazy Loading**: Load directory contents on-demand when expanded
3. **Caching**: Leverage React Query's caching for file contents
4. **Debounced Search**: Debounce file search input

### Security Considerations

The backend already implements security checks:
- Path traversal prevention in `LocalWorkspace.readFile()`
- Workspace boundary validation
- Task permission checking

### Future Enhancements

1. **File Icons**: Add file type icons based on extensions
2. **Syntax Highlighting**: Enhanced code highlighting with themes
3. **File Actions**: Copy path, open in editor (if integrated)
4. **Keyboard Navigation**: Arrow keys for tree navigation
5. **File Metadata**: Show file size, last modified date
6. **Breadcrumb Navigation**: Show current path with clickable segments

## Implementation Steps

### Backend Implementation Steps

1. **Add Type Definitions** (`shared/types/workspace.ts`):
   - Add `FileSearchResult` interface
   - Add `WorkspaceCapabilities` interface
   - Export types for use in both client and server

2. **Update Workspace Service** (`server/services/workspace.service.ts`):
   - Add `listFiles`, `readFile`, and `searchFiles` methods
   - Add capability checking helper methods
   - Implement proper error handling and workspace delegation

3. **Implement Local Workspace Methods** (`server/workspaces/local.ts`):
   - Add file listing with recursive directory traversal
   - Implement secure file reading with size limits
   - Add file search using ripgrep for performance
   - Implement security helper methods for path validation

4. **Add API Routes** (`server/routes/tasks.ts`):
   - Implement `GET /tasks/:taskId/files` for listing files
   - Implement `GET /tasks/:taskId/files/*` for reading files
   - Implement `POST /tasks/:taskId/search-files` for searching
   - Add proper authentication and error handling

5. **Test Backend Implementation**:
   - Test path traversal prevention
   - Verify file size limits work correctly
   - Test error scenarios (missing files, access denied)
   - Verify search functionality

### Frontend Implementation Steps

1. **Create Feature Module Structure**:
   - Create `client/src/features/file-viewer/` directory
   - Set up subdirectories for api, components, hooks, types
   - Create index.ts for exports

2. **Implement API Hooks**:
   - Create React Query hooks for file operations
   - Add proper TypeScript types
   - Implement error handling

3. **Build Core Components**:
   - File tree component with expand/collapse
   - File viewer with syntax highlighting
   - Loading skeletons for better UX
   - Error boundaries for isolation

4. **Update Query Keys**:
   - Add file-related query keys to `query-keys.ts`
   - Ensure proper cache invalidation

5. **Integrate with Task Detail**:
   - Add "Files" tab to task detail page
   - Update task UI store types
   - Implement resizable panels

6. **Add Polish**:
   - Implement keyboard navigation
   - Add file icons based on type
   - Optimize for large file trees
   - Add search functionality

7. **Write Tests**:
   - Component tests with React Testing Library
   - Hook tests for data fetching
   - Integration tests for file viewing flow

This implementation follows the project's established patterns for feature-based architecture, React Query for data fetching, and component composition for UI building.