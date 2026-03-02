/**
 * Content verification helpers for E2E tests.
 *
 * Provides utilities to search workspace files for expected content
 * after agent execution.
 */

import type { ApiClient } from "./api-client";

// =============================================================================
// Types
// =============================================================================

export interface FileMatch {
	path: string;
	content: string;
}

// =============================================================================
// Helpers
// =============================================================================

/**
 * Search workspace files for a string, checking common source paths first.
 *
 * @param api - API client instance
 * @param taskId - Task whose workspace to search
 * @param searchString - Text to look for inside files
 * @returns The first matching file, or null if not found
 */
export async function findFileWithContent(
	api: ApiClient,
	taskId: string,
	searchString: string,
): Promise<FileMatch | null> {
	// Check common source file locations first
	const candidates = [
		"src/pages/Index.tsx",
		"src/App.tsx",
		"src/App.jsx",
		"src/main.tsx",
		"index.html",
	];

	for (const filePath of candidates) {
		const content = await api.readWorkspaceFile(taskId, filePath);
		if (content && content.includes(searchString)) {
			return { path: filePath, content };
		}
	}

	// If not found in common paths, list src/ recursively
	const files = await api.listWorkspaceFiles(taskId, "src", true);
	for (const filePath of files) {
		if (filePath.endsWith(".tsx") || filePath.endsWith(".jsx") || filePath.endsWith(".html")) {
			const content = await api.readWorkspaceFile(taskId, filePath);
			if (content && content.includes(searchString)) {
				return { path: filePath, content };
			}
		}
	}

	return null;
}
