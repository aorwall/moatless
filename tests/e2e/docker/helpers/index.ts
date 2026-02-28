/**
 * Docker E2E Test Helpers
 *
 * Re-exports all helper modules for convenient importing.
 *
 * Usage:
 *   import { ApiClient, SSEEventCollector, waitForWorkspaceReady } from "../helpers";
 */

// API Client
export { ApiClient } from "./api-client";
export type {
	CreateRepositoryRequest,
	CreateVibeTaskRequest,
	GetMessagesResponse,
	GitDiffFile,
	GitDiffResponse,
	RepositoryResponse,
	ServerStatusResponse,
	TaskResponse,
	UiMessage,
	WorkspaceStatusFullResponse,
} from "./api-client";

// SSE Event Collector
export { SSEEventCollector } from "./sse-collector";
export type { SSEEvent, TaskLifecycleVerification } from "./sse-collector";

// Wait Helpers
export {
	waitForAgentComplete,
	waitForAgentIdleViaSSE,
	waitForCondition,
	waitForPreviewReady,
	waitForWorkspaceReady,
	waitForWorkspaceStatus,
} from "./wait-helpers";
export type { WaitOptions } from "./wait-helpers";

// Docker Helpers
export {
	cleanupTestContainers,
	getContainerLogs,
	getContainerNameForTask,
	getContainerNetworks,
	getContainerPort,
	getContainerState,
	imageExists,
	isDockerAvailable,
	isDockerComposeHealthy,
	listMoatlessContainers,
	removeContainer,
	saveContainerLogs,
	verifyContainerNetwork,
} from "./docker-helpers";
export type { ContainerNetwork, ContainerState } from "./docker-helpers";
