/**
 * Type-safe API client for E2E tests.
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
	UpdateRepositoryRequest,
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

export interface ListTasksFilters {
	limit?: number;
	offset?: number;
	status?: string;
	repositoryId?: string;
	tag?: string;
}

export interface ListTasksPagination {
	limit: number;
	offset: number;
	total: number;
	hasMore: boolean;
}

export interface ListTasksResponse {
	items: TaskResponse[];
	pagination: ListTasksPagination;
}

export interface FileTreeEntry {
	name: string;
	path: string;
	type: "file" | "directory";
	children?: FileTreeEntry[];
	gitStatus?: string;
}

export interface FileTreeResponse {
	tree: FileTreeEntry[];
}

export interface SearchByNameResult {
	files: Array<{ path: string; name: string }>;
}

// =============================================================================
// Plugin Repo Types
// =============================================================================

export interface PluginRepoResponse {
	id: string;
	name: string;
	gitUrl: string;
}

export interface CreatePluginRepoRequest {
	name: string;
	gitUrl: string;
}

export interface ChannelConfigResponse {
	id: string;
	provider: string;
	sourceId: string;
	sourceName?: string;
	repositoryId: string;
	routingMode: string;
	promptTemplate?: string;
	active: boolean;
	providerConfig?: Record<string, unknown>;
	ongoingTaskId?: string;
	responseConfigId?: string;
	createdBy: string;
	createdAt: string;
	updatedAt: string;
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
	UpdateRepositoryRequest,
	WorkspaceStatusFullResponse,
};

// =============================================================================
// Team Types
// =============================================================================

export interface TeamResponse {
	id: string;
	name: string;
	slug: string;
	description?: string;
	isDefault: boolean;
	createdAt: string;
	updatedAt: string;
}

export interface TeamMemberResponse {
	teamId: string;
	userId: string;
	role: string;
	createdAt: string;
}

export interface TeamCredentialResponse {
	provider: string;
	tokenType?: string;
	hasRefreshToken: boolean;
}

// =============================================================================
// API Client
// =============================================================================

export class ApiClient {
	constructor(
		private baseURL: string,
		private defaultHeaders: Record<string, string> = {},
	) {}

	/**
	 * Create a new ApiClient that sends ForwardAuth headers to simulate a specific user.
	 */
	asUser(identity: {
		userId: string;
		login?: string;
		role?: string;
		teamIds?: string[];
	}): ApiClient {
		const headers: Record<string, string> = {
			"X-User-Id": identity.userId,
			"X-User-Login": identity.login ?? identity.userId,
		};
		if (identity.role) {
			headers["X-User-Role"] = identity.role;
		}
		if (identity.teamIds?.length) {
			headers["X-Team-Ids"] = identity.teamIds.join(",");
		}
		return new ApiClient(this.baseURL, headers);
	}

	// =========================================================================
	// Private Fetch Helpers
	// =========================================================================

	private async fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
		const response = await fetch(url, {
			...options,
			headers: {
				"Content-Type": "application/json",
				Accept: "application/json",
				...this.defaultHeaders,
				...options?.headers,
			},
		});
		if (!response.ok) {
			const text = await response.text();
			throw new Error(`HTTP ${response.status}: ${text}`);
		}
		return response.json();
	}

	private async fetchOptionalJson<T>(url: string): Promise<T | null> {
		try {
			const response = await fetch(url, {
				headers: { Accept: "application/json", ...this.defaultHeaders },
			});
			if (!response.ok) {
				return null;
			}
			return response.json();
		} catch {
			return null;
		}
	}

	private async fetchRaw(
		url: string,
		options?: RequestInit & { throwOnError?: boolean },
	): Promise<Response> {
		const { throwOnError, ...fetchOptions } = options ?? {};
		const response = await fetch(url, {
			...fetchOptions,
			headers: {
				"Content-Type": "application/json",
				Accept: "application/json",
				...this.defaultHeaders,
				...fetchOptions?.headers,
			},
		});
		if (throwOnError && !response.ok) {
			const text = await response.text();
			throw new Error(`HTTP ${response.status}: ${text}`);
		}
		return response;
	}

	// =========================================================================
	// Repository Operations
	// =========================================================================

	async createRepository(config: CreateRepositoryRequest): Promise<RepositoryResponse> {
		return this.fetchJson<RepositoryResponse>(`${this.baseURL}/api/v1/repositories`, {
			method: "POST",
			body: JSON.stringify(config),
		});
	}

	async getRepository(id: string): Promise<RepositoryResponse> {
		return this.fetchJson<RepositoryResponse>(`${this.baseURL}/api/v1/repositories/${id}`);
	}

	/**
	 * Get repository and return the raw Response for error assertions.
	 */
	async getRepositoryRaw(id: string): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/repositories/${id}`);
	}

	async updateRepository(
		id: string,
		data: UpdateRepositoryRequest,
	): Promise<RepositoryResponse> {
		return this.fetchJson<RepositoryResponse>(`${this.baseURL}/api/v1/repositories/${id}`, {
			method: "PUT",
			body: JSON.stringify(data),
		});
	}

	async deleteRepository(id: string): Promise<void> {
		try {
			await this.fetchRaw(`${this.baseURL}/api/v1/repositories/${id}`, { method: "DELETE" });
		} catch {
			// Ignore
		}
	}

	/**
	 * Delete repository and return the raw Response for status code assertions.
	 */
	async deleteRepositoryRaw(id: string): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/repositories/${id}`, { method: "DELETE" });
	}

	/**
	 * Create repository and return the raw Response for error assertions.
	 */
	async createRepositoryRaw(config: CreateRepositoryRequest): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/repositories`, {
			method: "POST",
			body: JSON.stringify(config),
		});
	}

	/**
	 * Create repository with scope fields (not in generated types).
	 * Returns the full response body.
	 */
	async createRepositoryWithScope(config: {
		name: string;
		kind: string;
		scope: string;
		teamId?: string;
		remoteUrl?: string;
		description?: string;
	}): Promise<Record<string, unknown>> {
		return this.fetchJson<Record<string, unknown>>(
			`${this.baseURL}/api/v1/repositories`,
			{
				method: "POST",
				body: JSON.stringify(config),
			},
		);
	}

	/**
	 * List all repositories.
	 * NOTE: GET /api/v1/repositories returns RepositoryResponse[] (plain array).
	 */
	async listRepositories(): Promise<RepositoryResponse[]> {
		return this.fetchJson<RepositoryResponse[]>(`${this.baseURL}/api/v1/repositories`);
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
	 * Maps CreateVibeTaskRequest.templateId -> CreateTaskRequest.repositoryId
	 * and calls POST /api/v1/tasks (the actual backend endpoint).
	 */
	async createVibeTask(config: CreateVibeTaskRequest): Promise<TaskResponse> {
		const result = await this.fetchJson<{ taskId: string; status: string; task: TaskResponse }>(
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
		return this.fetchJson<TaskResponse>(`${this.baseURL}/api/v1/tasks/${taskId}`);
	}

	async deleteTask(taskId: string): Promise<void> {
		try {
			await this.fetchRaw(`${this.baseURL}/api/v1/tasks/${taskId}`, { method: "DELETE" });
		} catch {
			// Ignore
		}
	}

	/**
	 * Delete task and return the raw Response for status code assertions.
	 */
	async deleteTaskRaw(taskId: string): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/tasks/${taskId}`, { method: "DELETE" });
	}

	async stopTask(taskId: string): Promise<void> {
		try {
			await this.fetchRaw(`${this.baseURL}/api/v1/tasks/${taskId}/stop`, { method: "POST" });
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
		await this.fetchJson(`${this.baseURL}/api/v1/tasks/${taskId}/messages`, {
			method: "POST",
			body: JSON.stringify({
				message,
				resume: options?.resume ?? true,
				model: options?.model,
			}),
		});
	}

	/**
	 * Send a message and return the raw Response (for error assertions).
	 */
	async sendMessageRaw(
		taskId: string,
		message: string,
		options?: { resume?: boolean },
	): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/tasks/${taskId}/messages`, {
			method: "POST",
			body: JSON.stringify({
				message,
				resume: options?.resume ?? true,
			}),
		});
	}

	/**
	 * TASK-07: List tasks with optional filters.
	 */
	async listTasks(filters?: ListTasksFilters): Promise<ListTasksResponse> {
		const params = new URLSearchParams();
		if (filters?.limit != null) params.set("limit", String(filters.limit));
		if (filters?.offset != null) params.set("offset", String(filters.offset));
		if (filters?.status) params.set("status", filters.status);
		if (filters?.repositoryId) params.set("repository_id", filters.repositoryId);
		if (filters?.tag) params.set("tag", filters.tag);

		const qs = params.toString();
		const url = `${this.baseURL}/api/v1/tasks${qs ? `?${qs}` : ""}`;
		return this.fetchJson<ListTasksResponse>(url);
	}

	/**
	 * TASK-04: Update task metadata (name, description).
	 */
	async updateTaskMetadata(
		taskId: string,
		data: { name?: string; description?: string },
	): Promise<TaskResponse> {
		return this.fetchJson<TaskResponse>(`${this.baseURL}/api/v1/tasks/${taskId}`, {
			method: "PATCH",
			body: JSON.stringify(data),
		});
	}

	/**
	 * TASK-08: Close a task.
	 */
	async closeTask(taskId: string): Promise<TaskResponse> {
		return this.fetchJson<TaskResponse>(`${this.baseURL}/api/v1/tasks/${taskId}/close`, {
			method: "POST",
		});
	}

	/**
	 * Start a task (provisions sandbox).
	 */
	async startTask(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/v1/tasks/${taskId}/start`, {
			method: "POST",
			throwOnError: true,
		});
	}

	/**
	 * TASK-05: Set tags on a task.
	 */
	async setTaskTags(taskId: string, tags: string[]): Promise<unknown> {
		return this.fetchJson(`${this.baseURL}/api/v1/tasks/${taskId}/tags`, {
			method: "PUT",
			body: JSON.stringify({ tags }),
		});
	}

	/**
	 * TASK-06: Update task visibility.
	 */
	async updateVisibility(taskId: string, visibility: "private" | "public"): Promise<TaskResponse> {
		return this.fetchJson<TaskResponse>(`${this.baseURL}/api/v1/tasks/${taskId}/visibility`, {
			method: "PUT",
			body: JSON.stringify({ visibility }),
		});
	}

	/**
	 * Create a task with an invalid repository and return the raw Response.
	 */
	async createVibeTaskRaw(config: {
		repositoryId: string;
		message?: string;
	}): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/tasks`, {
			method: "POST",
			body: JSON.stringify(config),
		});
	}

	// =========================================================================
	// Workspace Operations
	// =========================================================================

	async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatusFullResponse> {
		return this.fetchJson<WorkspaceStatusFullResponse>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/status`,
		);
	}

	async getGitDiff(taskId: string): Promise<GitDiffResponse> {
		return this.fetchJson<GitDiffResponse>(`${this.baseURL}/api/v1/tasks/${taskId}/git-diff`);
	}

	// =========================================================================
	// Sandbox Operations
	// =========================================================================

	/**
	 * SANDBOX-02: Provision sandbox (idempotent).
	 */
	async provisionSandbox(taskId: string): Promise<unknown> {
		return this.fetchJson(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/provision`, {
			method: "POST",
		});
	}

	/**
	 * SANDBOX-03: Stop sandbox (scale to 0).
	 */
	async stopSandbox(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/stop`, {
			method: "POST",
			throwOnError: true,
		});
	}

	/**
	 * Start a stopped sandbox (via task start endpoint).
	 */
	async startSandbox(taskId: string): Promise<void> {
		await this.startTask(taskId);
	}

	/**
	 * SANDBOX-04: Restart sandbox (pod recreated, files persist).
	 */
	async restartSandbox(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/restart`, {
			method: "POST",
			throwOnError: true,
		});
	}

	/**
	 * SANDBOX-05: Cleanup sandbox (all resources removed).
	 */
	async cleanupSandbox(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/cleanup`, {
			method: "POST",
			throwOnError: true,
		});
	}

	// =========================================================================
	// Preview Operations
	// =========================================================================

	async getServerStatus(taskId: string): Promise<ServerStatusResponse> {
		return this.fetchJson<ServerStatusResponse>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/server/status`,
		);
	}

	async startPreview(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/start`, { method: "POST" });
	}

	async stopPreview(taskId: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/stop`, { method: "POST" });
	}

	// =========================================================================
	// File Operations
	// =========================================================================

	async readWorkspaceFile(taskId: string, filePath: string): Promise<string | null> {
		const params = new URLSearchParams({ path: filePath });
		const data = await this.fetchOptionalJson<{ content: string }>(
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
		const data = await this.fetchOptionalJson<{ files: string[] }>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/list?${params}`,
		);
		return data?.files ?? [];
	}

	/**
	 * FILE-03: Get file tree with optional git status overlay.
	 */
	async getFileTree(taskId: string, path?: string): Promise<FileTreeResponse> {
		const params = new URLSearchParams();
		if (path) params.set("path", path);

		const qs = params.toString();
		const url = `${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/tree${qs ? `?${qs}` : ""}`;
		return this.fetchJson<FileTreeResponse>(url);
	}

	/**
	 * FILE-05: Write a file to the workspace.
	 */
	async writeFile(taskId: string, path: string, content: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/write`, {
			method: "POST",
			body: JSON.stringify({ path, content }),
			throwOnError: true,
		});
	}

	/**
	 * FILE-04: Search files by name pattern.
	 */
	async searchFilesByName(taskId: string, query: string): Promise<SearchByNameResult> {
		return this.fetchJson<SearchByNameResult>(
			`${this.baseURL}/api/sandbox/v1/tasks/${taskId}/files/search-by-name`,
			{
				method: "POST",
				body: JSON.stringify({ query }),
			},
		);
	}

	// =========================================================================
	// Message Operations
	// =========================================================================

	async getTaskMessages(taskId: string): Promise<GetMessagesResponse> {
		return this.fetchJson<GetMessagesResponse>(
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
	// Channel Operations
	// =========================================================================

	async createChannelConfig(config: {
		provider: string;
		sourceId: string;
		sourceName?: string;
		repositoryId: string;
		routingMode: string;
		promptTemplate?: string;
		providerConfig?: Record<string, unknown>;
		responseConfigId?: string;
	}): Promise<ChannelConfigResponse> {
		return this.fetchJson<ChannelConfigResponse>(`${this.baseURL}/api/v1/channels/configs`, {
			method: "POST",
			body: JSON.stringify(config),
		});
	}

	async getChannelConfig(id: string): Promise<ChannelConfigResponse> {
		return this.fetchJson<ChannelConfigResponse>(`${this.baseURL}/api/v1/channels/configs/${id}`);
	}

	async updateChannelConfig(
		id: string,
		data: {
			sourceName?: string;
			routingMode?: string;
			promptTemplate?: string | null;
			active?: boolean;
			providerConfig?: Record<string, unknown>;
			responseConfigId?: string | null;
		},
	): Promise<ChannelConfigResponse> {
		return this.fetchJson<ChannelConfigResponse>(`${this.baseURL}/api/v1/channels/configs/${id}`, {
			method: "PUT",
			body: JSON.stringify(data),
		});
	}

	async deleteChannelConfig(id: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/v1/channels/configs/${id}`, {
			method: "DELETE",
			throwOnError: true,
		});
	}

	async deleteChannelConfigRaw(id: string): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/channels/configs/${id}`, {
			method: "DELETE",
		});
	}

	async listChannelConfigs(filters?: {
		provider?: string;
		repositoryId?: string;
	}): Promise<ChannelConfigResponse[]> {
		const params = new URLSearchParams();
		if (filters?.provider) params.set("provider", filters.provider);
		if (filters?.repositoryId) params.set("repositoryId", filters.repositoryId);

		const qs = params.toString();
		const url = `${this.baseURL}/api/v1/channels/configs${qs ? `?${qs}` : ""}`;
		return this.fetchJson<ChannelConfigResponse[]>(url);
	}

	/**
	 * Send a message via webhook inbound endpoint.
	 */
	async sendWebhookMessage(
		configId: string,
		payload: { text: string; threadId?: string; userId?: string; metadata?: Record<string, unknown> },
	): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/channels/webhook/${configId}`, {
			method: "POST",
			body: JSON.stringify(payload),
		});
	}

	/**
	 * Send a raw webhook message (for testing invalid payloads).
	 */
	async sendWebhookMessageRaw(
		configId: string,
		payload: unknown,
		headers?: Record<string, string>,
	): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/channels/webhook/${configId}`, {
			method: "POST",
			body: JSON.stringify(payload),
			headers,
		});
	}

	// =========================================================================
	// Team Operations
	// =========================================================================

	async createTeam(req: {
		name: string;
		slug: string;
		description?: string;
	}): Promise<TeamResponse> {
		const response = await this.fetchRaw(`${this.baseURL}/api/v1/teams`, {
			method: "POST",
			body: JSON.stringify(req),
			throwOnError: true,
		});
		return response.json();
	}

	async createTeamRaw(req: {
		name: string;
		slug: string;
		description?: string;
	}): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/teams`, {
			method: "POST",
			body: JSON.stringify(req),
		});
	}

	async getTeam(id: string): Promise<TeamResponse> {
		return this.fetchJson<TeamResponse>(`${this.baseURL}/api/v1/teams/${id}`);
	}

	async updateTeam(
		id: string,
		req: { name: string; description?: string },
	): Promise<TeamResponse> {
		return this.fetchJson<TeamResponse>(`${this.baseURL}/api/v1/teams/${id}`, {
			method: "PUT",
			body: JSON.stringify(req),
		});
	}

	async deleteTeam(id: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/v1/teams/${id}`, {
			method: "DELETE",
			throwOnError: true,
		});
	}

	async deleteTeamRaw(id: string): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/teams/${id}`, {
			method: "DELETE",
		});
	}

	async listTeams(): Promise<TeamResponse[]> {
		return this.fetchJson<TeamResponse[]>(`${this.baseURL}/api/v1/teams`);
	}

	async addTeamMember(
		teamId: string,
		userId: string,
		role?: string,
	): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/teams/${teamId}/members`,
			{
				method: "POST",
				body: JSON.stringify({ userId, role }),
				throwOnError: true,
			},
		);
	}

	async removeTeamMember(teamId: string, userId: string): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/teams/${teamId}/members/${userId}`,
			{ method: "DELETE", throwOnError: true },
		);
	}

	async listTeamMembers(teamId: string): Promise<TeamMemberResponse[]> {
		return this.fetchJson<TeamMemberResponse[]>(
			`${this.baseURL}/api/v1/teams/${teamId}/members`,
		);
	}

	async upsertTeamCredential(
		teamId: string,
		provider: string,
		req: {
			accessToken: string;
			refreshToken?: string;
			tokenType?: string;
		},
	): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/teams/${teamId}/credentials/${provider}`,
			{
				method: "PUT",
				body: JSON.stringify(req),
				throwOnError: true,
			},
		);
	}

	async upsertTeamCredentialRaw(
		teamId: string,
		provider: string,
		req: {
			accessToken: string;
			refreshToken?: string;
			tokenType?: string;
		},
	): Promise<Response> {
		return this.fetchRaw(
			`${this.baseURL}/api/v1/teams/${teamId}/credentials/${provider}`,
			{
				method: "PUT",
				body: JSON.stringify(req),
			},
		);
	}

	async deleteTeamCredential(
		teamId: string,
		provider: string,
	): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/teams/${teamId}/credentials/${provider}`,
			{ method: "DELETE", throwOnError: true },
		);
	}

	async listTeamCredentials(
		teamId: string,
	): Promise<TeamCredentialResponse[]> {
		return this.fetchJson<TeamCredentialResponse[]>(
			`${this.baseURL}/api/v1/teams/${teamId}/credentials`,
		);
	}

	// =========================================================================
	// Plugin Repo Operations
	// =========================================================================

	async createPluginRepo(req: CreatePluginRepoRequest): Promise<PluginRepoResponse> {
		return this.fetchJson<PluginRepoResponse>(`${this.baseURL}/api/v1/plugin-repos`, {
			method: "POST",
			body: JSON.stringify(req),
		});
	}

	async listPluginRepos(): Promise<PluginRepoResponse[]> {
		return this.fetchJson<PluginRepoResponse[]>(`${this.baseURL}/api/v1/plugin-repos`);
	}

	async deletePluginRepo(id: string): Promise<void> {
		await this.fetchRaw(`${this.baseURL}/api/v1/plugin-repos/${id}`, {
			method: "DELETE",
			throwOnError: true,
		});
	}

	async listRepoPlugins(repoId: string): Promise<PluginRepoResponse[]> {
		return this.fetchJson<PluginRepoResponse[]>(
			`${this.baseURL}/api/v1/repositories/${repoId}/plugins`,
		);
	}

	async linkPluginToRepo(repoId: string, pluginId: string): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/repositories/${repoId}/plugins/${pluginId}`,
			{ method: "POST", throwOnError: true },
		);
	}

	async unlinkPluginFromRepo(repoId: string, pluginId: string): Promise<void> {
		await this.fetchRaw(
			`${this.baseURL}/api/v1/repositories/${repoId}/plugins/${pluginId}`,
			{ method: "DELETE", throwOnError: true },
		);
	}

	// =========================================================================
	// Auth Verify
	// =========================================================================

	/**
	 * Call the ForwardAuth verify endpoint and return the raw Response.
	 * Optionally pass extra headers (e.g. X-Forwarded-Host for preview access).
	 */
	async verifyAuth(extraHeaders?: Record<string, string>): Promise<Response> {
		return this.fetchRaw(`${this.baseURL}/api/v1/auth/verify`, {
			headers: extraHeaders,
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
