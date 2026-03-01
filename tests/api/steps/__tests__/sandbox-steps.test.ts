/**
 * Unit tests for waitForSandboxReady — stability re-check behavior.
 *
 * Verifies that:
 * - Returns { ready: false, status: null } when waitForWorkspaceReady returns false
 * - Returns { ready: false, status } when re-poll after 3s shows non-ready status
 * - Returns { ready: true, status } when re-poll confirms still ready
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import type { WorkspaceStatusFullResponse } from "@moatless/api-client";
import type { ApiClient } from "../../../shared/api-client";

// Mock wait-helpers before importing the module under test
vi.mock("../../../shared/wait-helpers", () => ({
	waitForWorkspaceReady: vi.fn(),
	waitForWorkspaceStatus: vi.fn(),
}));

// Must import after vi.mock
const { waitForWorkspaceReady: mockWaitForWorkspaceReady } = await import(
	"../../../shared/wait-helpers"
);
const { waitForSandboxReady } = await import("../sandbox.steps");

function makeStatus(
	overrides: Partial<WorkspaceStatusFullResponse> = {},
): WorkspaceStatusFullResponse {
	return {
		environmentStatus: "ready",
		environmentError: null,
		workspaceReady: true,
		canExecute: true,
		agentState: "idle",
		servers: [],
		workspaceState: "ready",
		...overrides,
	} as WorkspaceStatusFullResponse;
}

describe("waitForSandboxReady", () => {
	let mockApi: ApiClient;

	beforeEach(() => {
		vi.clearAllMocks();
		// Replace setTimeout-based delay with immediate resolution
		vi.useFakeTimers({ shouldAdvanceTime: true });
		mockApi = {
			getWorkspaceStatus: vi.fn(),
		} as unknown as ApiClient;
	});

	it("returns { ready: false, status: null } when workspace never becomes ready", async () => {
		vi.mocked(mockWaitForWorkspaceReady).mockResolvedValue(false);

		const result = await waitForSandboxReady(mockApi, "task-1");

		expect(result.ready).toBe(false);
		expect(result.status).toBeNull();
		expect(mockApi.getWorkspaceStatus).not.toHaveBeenCalled();
	});

	it("returns { ready: true, status } when re-poll confirms ready", async () => {
		vi.mocked(mockWaitForWorkspaceReady).mockResolvedValue(true);
		const readyStatus = makeStatus({ environmentStatus: "ready" });
		vi.mocked(mockApi.getWorkspaceStatus).mockResolvedValue(readyStatus);

		const result = await waitForSandboxReady(mockApi, "task-1");

		expect(result.ready).toBe(true);
		expect(result.status).toEqual(readyStatus);
		expect(mockApi.getWorkspaceStatus).toHaveBeenCalledWith("task-1");
	});

	it("returns { ready: false, status } when re-poll shows non-ready status", async () => {
		vi.mocked(mockWaitForWorkspaceReady).mockResolvedValue(true);
		const errorStatus = makeStatus({
			environmentStatus: "error",
			environmentError: "CrashLoopBackOff",
		});
		vi.mocked(mockApi.getWorkspaceStatus).mockResolvedValue(errorStatus);

		const result = await waitForSandboxReady(mockApi, "task-1");

		expect(result.ready).toBe(false);
		expect(result.status).toEqual(errorStatus);
	});

	it("returns { ready: false, status } when re-poll shows initializing (transient ready)", async () => {
		vi.mocked(mockWaitForWorkspaceReady).mockResolvedValue(true);
		const initStatus = makeStatus({
			environmentStatus: "initializing",
		});
		vi.mocked(mockApi.getWorkspaceStatus).mockResolvedValue(initStatus);

		const result = await waitForSandboxReady(mockApi, "task-1");

		expect(result.ready).toBe(false);
		expect(result.status).toEqual(initStatus);
	});
});
