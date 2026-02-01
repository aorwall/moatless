/**
 * Type-safe API client wrapper for Docker E2E tests.
 *
 * Provides a clean interface for interacting with the Moatless Vibe API
 * during E2E testing against a docker-compose environment.
 */

import type { APIRequestContext } from "@playwright/test";

// =============================================================================
// Types
// =============================================================================

export interface CreateRepositoryRequest {
	kind: "remote" | "template";
	name: string;
	remoteUrl: string;
	provider?: "github" | "gitlab";
	description?: string;
	codeExecution?: {
		dockerImage?: string;
		setupCommands?: string[];
		envVars?: Record<string, string>;
	};
	previewConfig?: {
		serverStartCommands?: string[];
		buildCommand?: string;
		buildOutputDirectory?: string;
		startupTimeoutSeconds?: number;
		autoReload?: boolean;
	};
}

export interface Repository {
	id: string;
	kind: "remote" | "template";
	name: string;
	description?: string;
	codeExecution: {
		dockerImage?: string;
		setupCommands?: string[];
		envVars?: Record<string, string>;
	};
	previewConfig?: {
		serverStartCommands?: string[];
		buildCommand?: string;
		buildOutputDirectory?: string;
		startupTimeoutSeconds?: number;
	};
	remote?: {
		url: string;
		provider: string;
		defaultBranch?: string;
	};
	createdAt: string;
	updatedAt: string;
}

export interface CreateTaskRequest {
	agentType: string;
	repositoryId: string;
	message?: string;
	start?: boolean;
	model?: string;
	templateId?: string;
}

export interface Task {
	taskId: string;
	shortId?: string;
	repositoryId: string;
	agentType: string;
	status: string;
	numTurns: number;
	totalCostUsd: number;
	siteUrl?: string;
	createdAt: string;
	lastMessageAt: string;
}

export interface WorkspaceStatus {
	environmentStatus: string;
	workspaceReady: boolean;
	executionMode?: string;
	agentState?: string;
	preview?: {
		status: string;
		url?: string;
		serverPort?: number;
	};
}

export interface GitDiffFile {
	oldFile?: string;
	newFile?: string;
	hunks: string[];
}

export interface GitDiffResponse {
	diff?: string;
	files: GitDiffFile[];
}

export interface PreviewStatus {
	status: "stopped" | "starting" | "started" | "failed";
	url?: string;
	serverPort?: number;
	error?: string;
}

// =============================================================================
// API Client
// =============================================================================

export class ApiClient {
	constructor(
		private request: APIRequestContext,
		private baseURL: string,
	) {}

	// =========================================================================
	// Repository Operations
	// =========================================================================

	/**
	 * Create a new repository
	 */
	async createRepository(config: CreateRepositoryRequest): Promise<Repository> {
		const response = await this.request.post(`${this.baseURL}/api/v1/repositories`, {
			data: config,
		});

		if (!response.ok()) {
			const text = await response.text();
			throw new Error(`Failed to create repository: ${response.status()} ${text}`);
		}

		return response.json();
	}

	/**
	 * Get a repository by ID
	 */
	async getRepository(id: string): Promise<Repository> {
		const response = await this.request.get(`${this.baseURL}/api/v1/repositories/${id}`);

		if (!response.ok()) {
			throw new Error(`Failed to get repository: ${response.status()}`);
		}

		return response.json();
	}

	/**
	 * Delete a repository
	 */
	async deleteRepository(id: string): Promise<void> {
		const response = await this.request.delete(`${this.baseURL}/api/v1/repositories/${id}`);

		if (!response.ok() && response.status() !== 404) {
			throw new Error(`Failed to delete repository: ${response.status()}`);
		}
	}

	/**
	 * List all repositories
	 */
	async listRepositories(): Promise<Repository[]> {
		const response = await this.request.get(`${this.baseURL}/api/v1/repositories`);

		if (!response.ok()) {
			throw new Error(`Failed to list repositories: ${response.status()}`);
		}

		const data = await response.json();
		return data.repositories || [];
	}

	/**
	 * Get the first template repository
	 */
	async getTemplateRepository(): Promise<Repository | null> {
		const repositories = await this.listRepositories();
		return repositories.find((r) => r.kind === "template") || null;
	}

	// =========================================================================
	// Task Operations
	// =========================================================================

	/**
	 * Create a new task
	 */
	async createTask(config: CreateTaskRequest): Promise<Task> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks`, {
			data: config,
		});

		if (!response.ok()) {
			const text = await response.text();
			throw new Error(`Failed to create task: ${response.status()} ${text}`);
		}

		return response.json();
	}

	/**
	 * Create a vibe task from a template
	 */
	async createVibeTask(config: {
		agentType: string;
		templateId: string;
		message?: string;
		start?: boolean;
	}): Promise<Task> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks/vibe`, {
			data: config,
		});

		if (!response.ok()) {
			const text = await response.text();
			throw new Error(`Failed to create vibe task: ${response.status()} ${text}`);
		}

		return response.json();
	}

	/**
	 * Get a task by ID
	 */
	async getTask(taskId: string): Promise<Task> {
		const response = await this.request.get(`${this.baseURL}/api/v1/tasks/${taskId}`);

		if (!response.ok()) {
			throw new Error(`Failed to get task: ${response.status()}`);
		}

		return response.json();
	}

	/**
	 * Delete a task
	 */
	async deleteTask(taskId: string): Promise<void> {
		const response = await this.request.delete(`${this.baseURL}/api/v1/tasks/${taskId}`);

		if (!response.ok() && response.status() !== 404) {
			throw new Error(`Failed to delete task: ${response.status()}`);
		}
	}

	/**
	 * Stop a running task/agent
	 */
	async stopTask(taskId: string): Promise<void> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks/${taskId}/stop`);

		// Accept various success codes - task might not be running
		if (!response.ok() && response.status() !== 404 && response.status() !== 409) {
			throw new Error(`Failed to stop task: ${response.status()}`);
		}
	}

	/**
	 * Send a message to start or continue agent execution
	 */
	async sendMessage(
		taskId: string,
		message: string,
		options?: {
			resume?: boolean;
			model?: string;
		},
	): Promise<void> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks/${taskId}/messages`, {
			data: {
				message,
				resume: options?.resume ?? true,
				model: options?.model,
			},
		});

		if (!response.ok()) {
			const text = await response.text();
			throw new Error(`Failed to send message: ${response.status()} ${text}`);
		}
	}

	// =========================================================================
	// Workspace Operations
	// =========================================================================

	/**
	 * Get workspace status for a task
	 */
	async getWorkspaceStatus(taskId: string): Promise<WorkspaceStatus> {
		const response = await this.request.get(
			`${this.baseURL}/api/v1/tasks/${taskId}/workspace-status`,
		);

		if (!response.ok()) {
			throw new Error(`Failed to get workspace status: ${response.status()}`);
		}

		return response.json();
	}

	/**
	 * Get git diff from workspace
	 */
	async getGitDiff(taskId: string): Promise<GitDiffResponse> {
		const response = await this.request.get(`${this.baseURL}/api/v1/tasks/${taskId}/git-diff`);

		if (!response.ok()) {
			throw new Error(`Failed to get git diff: ${response.status()}`);
		}

		return response.json();
	}

	// =========================================================================
	// Preview Operations
	// =========================================================================

	/**
	 * Get preview server status
	 */
	async getPreviewStatus(taskId: string): Promise<PreviewStatus> {
		const response = await this.request.get(
			`${this.baseURL}/api/v1/tasks/${taskId}/server/status`,
		);

		if (!response.ok()) {
			throw new Error(`Failed to get preview status: ${response.status()}`);
		}

		return response.json();
	}

	/**
	 * Start preview server
	 */
	async startPreview(taskId: string): Promise<void> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks/${taskId}/server/start`);

		// Accept 200 OK or 202 Accepted (async operation)
		if (!response.ok() && response.status() !== 202) {
			throw new Error(`Failed to start preview: ${response.status()}`);
		}
	}

	/**
	 * Stop preview server
	 */
	async stopPreview(taskId: string): Promise<void> {
		const response = await this.request.post(`${this.baseURL}/api/v1/tasks/${taskId}/server/stop`);

		if (!response.ok() && response.status() !== 404) {
			throw new Error(`Failed to stop preview: ${response.status()}`);
		}
	}

	// =========================================================================
	// Health Check
	// =========================================================================

	/**
	 * Check if the backend is healthy
	 */
	async isHealthy(): Promise<boolean> {
		try {
			const response = await this.request.get(`${this.baseURL}/health`);
			if (!response.ok()) return false;

			const data = await response.json();
			return data.status === "ok";
		} catch {
			return false;
		}
	}
}
