/**
 * Content verification helpers for E2E tests.
 *
 * Provides utilities to search workspace files for expected content
 * after agent execution.
 */

import type { MoatApi } from './moat-api'

// =============================================================================
// Types
// =============================================================================

export interface FileMatch {
  path: string
  content: string
}

// =============================================================================
// Helpers
// =============================================================================

/**
 * Find a file the agent was asked to create, and check what is in it.
 *
 * The name is what the test asked for, so the name is what it looks for —
 * where in the tree the agent put it is the agent's business. Nothing here
 * assumes a particular repository.
 */
export async function findNamedFileWithContent(
  api: MoatApi,
  taskId: string,
  fileName: string,
  searchString: string,
): Promise<FileMatch | null> {
  const found = await api.searchFilesByName(taskId, fileName)
  for (const file of found.files ?? []) {
    const content = await api.readWorkspaceFile(taskId, file.path)
    if (content?.includes(searchString)) {
      return { path: file.path, content }
    }
  }
  return null
}

/**
 * Search workspace files for a string, checking common source paths first.
 *
 * Assumes a front-end repository laid out the way the `Hello World UI`
 * template is — the tests that use it ask the agent to edit "the main page",
 * which only means anything in such a repository.
 *
 * @returns The first matching file, or null if not found
 */
export async function findFileWithContent(
  api: MoatApi,
  taskId: string,
  searchString: string,
): Promise<FileMatch | null> {
  // Check common source file locations first
  const candidates = [
    'src/pages/Index.tsx',
    'src/App.tsx',
    'src/App.jsx',
    'src/main.tsx',
    'index.html',
  ]

  for (const filePath of candidates) {
    const content = await api.readWorkspaceFile(taskId, filePath)
    if (content?.includes(searchString)) {
      return { path: filePath, content }
    }
  }

  // If not found in common paths, list src/ recursively
  const files = await api.listWorkspaceFiles(taskId, 'src', true)
  for (const filePath of files) {
    if (filePath.endsWith('.tsx') || filePath.endsWith('.jsx') || filePath.endsWith('.html')) {
      const content = await api.readWorkspaceFile(taskId, filePath)
      if (content?.includes(searchString)) {
        return { path: filePath, content }
      }
    }
  }

  return null
}
