/**
 * Sandbox step functions (SANDBOX-01 through SANDBOX-06).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	WorkspaceStatusFullResponse,
} from "../../shared/api-client";
import { waitForWorkspaceReady, waitForWorkspaceStatus } from "../../shared/wait-helpers";

// =============================================================================
// SANDBOX-01: Wait for sandbox ready
// =============================================================================

export async function waitForSandboxReady(
	api: ApiClient,
	taskId: string,
	timeoutMs = 180000,
): Promise<{ ready: boolean; status: WorkspaceStatusFullResponse | null }> {
	const ready = await waitForWorkspaceReady(api, taskId, { timeout: timeoutMs });
	if (!ready) {
		return { ready: false, status: null };
	}

	// Stability check: wait briefly and re-verify to catch transient ready
	// states during crash loops (sandbox.ready → sandbox.error cycling)
	await new Promise((resolve) => setTimeout(resolve, 3000));
	const status = await api.getWorkspaceStatus(taskId);

	if (status.environmentStatus !== "ready") {
		console.log(
			`[waitForSandboxReady] Sandbox was briefly ready but reverted to ${status.environmentStatus}`,
		);
		if (status.environmentError) {
			console.log(
				`[waitForSandboxReady] Error: ${status.environmentError}`,
			);
		}
		return { ready: false, status };
	}

	return { ready: true, status };
}

// =============================================================================
// SANDBOX-02: Idempotent re-provision
// =============================================================================

export async function provisionSandbox(
	api: ApiClient,
	taskId: string,
): Promise<{ result: unknown; status: WorkspaceStatusFullResponse }> {
	const result = await api.provisionSandbox(taskId);
	const status = await api.getWorkspaceStatus(taskId);
	return { result, status };
}

// =============================================================================
// SANDBOX-03: Stop sandbox
// =============================================================================

export async function stopSandbox(
	api: ApiClient,
	taskId: string,
	waitTimeoutMs = 60000,
): Promise<WorkspaceStatusFullResponse | null> {
	await api.stopSandbox(taskId);
	return waitForWorkspaceStatus(api, taskId, "stopped", { timeout: waitTimeoutMs });
}

// =============================================================================
// SANDBOX-03.1: Start stopped sandbox
// =============================================================================

export async function startSandbox(
	api: ApiClient,
	taskId: string,
	waitTimeoutMs = 180000,
): Promise<{ ready: boolean; status: WorkspaceStatusFullResponse | null }> {
	await api.startSandbox(taskId);
	const ready = await waitForWorkspaceReady(api, taskId, { timeout: waitTimeoutMs });

	let status: WorkspaceStatusFullResponse | null = null;
	if (ready) {
		status = await api.getWorkspaceStatus(taskId);
	}

	return { ready, status };
}

// =============================================================================
// SANDBOX-04: Restart sandbox
// =============================================================================

export async function restartSandbox(
	api: ApiClient,
	taskId: string,
	waitTimeoutMs = 180000,
): Promise<{ ready: boolean; status: WorkspaceStatusFullResponse | null }> {
	await api.restartSandbox(taskId);
	const ready = await waitForWorkspaceReady(api, taskId, { timeout: waitTimeoutMs });

	let status: WorkspaceStatusFullResponse | null = null;
	if (ready) {
		status = await api.getWorkspaceStatus(taskId);
	}

	return { ready, status };
}

// =============================================================================
// SANDBOX-05: Cleanup sandbox
// =============================================================================

export async function cleanupSandbox(
	api: ApiClient,
	taskId: string,
	waitTimeoutMs = 60000,
): Promise<WorkspaceStatusFullResponse | null> {
	await api.cleanupSandbox(taskId);
	return waitForWorkspaceStatus(api, taskId, "not_created", { timeout: waitTimeoutMs });
}
