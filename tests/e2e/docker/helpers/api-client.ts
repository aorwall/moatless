/**
 * Type-safe API client wrapper for Docker E2E tests.
 *
 * Uses native fetch instead of Playwright's request fixture to avoid
 * fixture scoping issues (beforeAll request can't be reused in tests).
 *
 * Uses generated types from @moatless/api-client for type safety.
 */

import type {
	CreateRepositoryRequest,
	CreateVibeTaskRequest,
	GetMessagesResponse,
	RepositoryResponse,
	ServerStatusResponse,
	TaskResponse,
	UiMessage,
	WorkspaceStatusFullResponse,
} from "@moatless/api-client";

// =============================================================================
// Types (only those not available in @moatless/api-client)
// =============================================================================

export interface GitDiffFile {
	oldFile?: string;
	newFile?: string;
	hunks: string[];
}

export interface GitDiffResponse {
	diff?: string;
	files: GitDiffFile[];
}

// Re-export generated types for convenience
export type {
	CreateRepositoryRequest,
	CreateVibeTaskRequest,
	GetMessagesResponse,
	RepositoryResponse,
	ServerStatusResponse,
	TaskResponse,
	UiMessage,
	WorkspaceStatusFullResponse,
};

// =============================================================================
// Fetch helpers
// =============================================================================

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
	const response = await fetch(url, {
		...options,
		headers: {
			"Content-Type": "application/json",
			Accept: "application/json",
			...options?.headers,
		},
	});
	if (!response.ok) {
		const text = await response.text();
		throw new Error(`HTTP ${response.status}: ${text}`);
	}
	return response.json();
}

async function fetchOptionalJson<T>(url: string): Promise<T | null> {
	try {
		const response = await fetch(url, {
			headers: { Accept: "application/json" },
		});
		if (!response.ok) {
			return null;
		}
		return response.json();
	} catch {
		return null;
	}
}

// =============================================================================
// API Client
// =============================================================================

export class ApiClient {
	constructor(private baseURL: string) {}

	// =========================================================================
	// Repository Operations
	// =========================================================================

	async createRepository(config: CreateRepositoryRequest): Promise<RepositoryResponse> {
		return fetchJson<RepositoryResponse>(`${this.baseURL}/api/v1/repositories`, {
			method: "POST",
			body: JSON.stringify(config),
		});
	}

	async getRepository(id: string): Promise<RepositoryResponse> {
		return fetchJson<RepositoryResponse>(`${this.baseURL}/api/v1/repositories/${id}`);
	}

	async deleteRepository(id: string): Promise<void> {
		try {
			await fetch(`${this.baseURL}/api/v1/repositories/${id}`, { method: "DELETE" });
		} catch {
			// Ignore
		}
	}

	/**
	 * List all repositories.
	 * NOTE: GET /api/v1/repositories returns RepositoryResponse[] (plain array).
	 */
	async listRepositories(): Promise<RepositoryResponse[]> {
		return fetchJson<RepositoryResponse[]>(`${this.baseURL}/api/v1/repositories`);
	}

	async getTemplateRepository(): Promise<RepositoryResponse | null> {
		const repositories = await this.listRepositories();
		return repositories.find((r) => r.kind === "template") || null;
	}

	// =========================================================================
	// Task Operations
	// =========================================================================

	/**
	 * Create a vibe task from a template.
	 *
	 * Maps CreateVibeTaskRequest.templateId → CreateTaskRequest.repositoryId
	 * and calls POST /api/v1/tasks (the actual backend endpoint).
	 */
	async createVibeTask(config: CreateVibeTaskRequest): Promise<TaskResponse> {
		const result = await fetchJson<{ taskId: string; status: string; task: TaskResponse }>(
			`${this.baseURL}/api/v1/tasks`,
			{
				method: "POST",
				body: JSON.stringify({
					repositoryId: config.templateId,
					message: config.message,
					name: config.name,
				}),
			},
		);
		return result.task;
	}

	async getTask(taskId: string): Promise<TaskResponse> {
		return fetchJson<TaskResponse>(`${this.baseURL}/api/v1/tasks/${taskId}`);
	}

	async deleteTask(taskId: string): Promise<void> {
		try {
			await fetch(`${this.baseURL}/api/v1/tasks/${taskId}`, { method: "DELETE" });
		} catch {
			// Ignore
		}
	}

	async stopTask(taskId: string): Promise<void> {
		try {
			await fetch(`${this.baseURL}/api/v1/tasks/${taskId}/stop`, { method: "POST" });
		} catch {
			// Ignore - task might not be running
		}
	}

	async sendMessage(
		taskId: string,
		message: string,
		options?: {
			resume?: boolean;
			model?: string;
		},
	): Promise<void> {
		await fetchJson(`${this.baseURL}/api/v1/tasks/${taskId}/messages`, {
			method: "POST",
			body: JSON.stringify({
				message,
				resume: options?.resume ?? true,
				model: options?.model,
			}),
		});
	}

	// =========================================================================
	// Workspace Operations
	// =========================================================================

	async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusFullResponse> {
		return fetchJson<WorkspaceStatusFullResponse>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/status`,
		);
	}

	async getGitDiff(taskId: string): Promise<GitDiffResponse> {
		return fetchJson<GitDiffResponse>(`${this.baseURL}/api/v1/tasks/${taskId}/git-diff`);
	}

	// =========================================================================
	// Preview Operations
	// =========================================================================

	async getServerStatus(taskId: string): Promise<ServerStatusResponse> {
		return fetchJson<ServerStatusResponse>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/server/status`,
		);
	}

	async startPreview(taskId: string): Promise<void> {
		await fetch(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/start`, { method: "POST" });
	}

	async stopPreview(taskId: string): Promise<void> {
		await fetch(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/stop`, { method: "POST" });
	}

	// =========================================================================
	// File Operations
	// =========================================================================

	async readWorkspaceFile(taskId: string, filePath: string): Promise<string | null> {
		const params = new URLSearchParams({ path: filePath });
		const data = await fetchOptionalJson<{ content: string }>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/read?${params}`,
		);
		return data?.content ?? null;
	}

	async listWorkspaceFiles(
		taskId: string,
		dirPath: string,
		recursive = false,
	): Promise<string[]> {
		const params = new URLSearchParams({
			path: dirPath,
			recursive: String(recursive),
		});
		const data = await fetchOptionalJson<{ files: string[] }>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/list?${params}`,
		);
		return data?.files ?? [];
	}

	// =========================================================================
	// Message Operations
	// =========================================================================

	async getTaskMessages(taskId: string): Promise<GetMessagesResponse> {
		return fetchJson<GetMessagesResponse>(
			`${this.baseURL}/api/v1/tasks/${taskId}/messages`,
		);
	}

	// =========================================================================
	// Repository Helpers
	// =========================================================================

	async ensureTemplateRepository(): Promise<RepositoryResponse> {
		const repositories = await this.listRepositories();
		const template = repositories.find((r) => r.kind === "template");
		if (template) {
			return template;
		}

		if (repositories.length > 0) {
			return repositories[0];
		}

		return this.createRepository({
			name: "Hello World UI",
			kind: "template",
			remoteUrl: "https://github.com/aorwall/hello-world-ui.git",
		});
	}

	// =========================================================================
	// Health Check
	// =========================================================================

	async isHealthy(): Promise<boolean> {
		try {
			const response = await fetch(`${this.baseURL}/health`);
			if (!response.ok) return false;
			const data = await response.json();
			return data.status === "ok";
		} catch {
			return false;
		}
	}
}
