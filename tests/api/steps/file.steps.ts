/**
 * File step functions (FILE-01 through FILE-06).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	FileTreeResponse,
	SearchByNameResult,
} from "../../shared/api-client";
import { findFileWithContent } from "../../shared/content-helpers";
import type { FileMatch } from "../../shared/content-helpers";

// =============================================================================
// FILE-01: List workspace files
// =============================================================================

export async function listFiles(
	api: ApiClient,
	taskId: string,
	dirPath = ".",
	recursive = false,
): Promise<string[]> {
	return api.listWorkspaceFiles(taskId, dirPath, recursive);
}

// =============================================================================
// FILE-02: Read workspace file
// =============================================================================

export async function readFile(
	api: ApiClient,
	taskId: string,
	filePath: string,
): Promise<string | null> {
	return api.readWorkspaceFile(taskId, filePath);
}

// =============================================================================
// FILE-03: File tree with git status overlay
// =============================================================================

export async function getFileTree(
	api: ApiClient,
	taskId: string,
	path?: string,
): Promise<FileTreeResponse> {
	return api.getFileTree(taskId, path);
}

// =============================================================================
// FILE-04: Search files by name
// =============================================================================

export async function searchFilesByName(
	api: ApiClient,
	taskId: string,
	query: string,
): Promise<SearchByNameResult> {
	return api.searchFilesByName(taskId, query);
}

// =============================================================================
// FILE-05: Write file to workspace
// =============================================================================

export async function writeFile(
	api: ApiClient,
	taskId: string,
	path: string,
	content: string,
): Promise<void> {
	return api.writeFile(taskId, path, content);
}

// =============================================================================
// FILE-06: Find file with content (after agent changes)
// =============================================================================

export async function findFileWithExpectedContent(
	api: ApiClient,
	taskId: string,
	searchString: string,
): Promise<FileMatch | null> {
	return findFileWithContent(api, taskId, searchString);
}
