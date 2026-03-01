/**
 * Unit tests for waitForWorkspaceReady — environmentError fast-fail behavior.
 *
 * Verifies that:
 * - Returns false immediately when environmentError is set
 * - Returns true when environmentStatus === "ready"
 * - Calls onProgress with the error message
 */

import { describe, it, expect, vi } from "vitest";
import { waitForWorkspaceReady } from "../wait-helpers";
import type { ApiClient } from "../api-client";
import type { WorkspaceStatusFullResponse } from "@moatless/api-client";

function makeStatus(
	overrides: Partial<WorkspaceStatusFullResponse> = {},
): WorkspaceStatusFullResponse {
	return {
		environmentStatus: "initializing",
		environmentError: null,
		workspaceReady: false,
		canExecute: false,
		agentState: "none",
		servers: [],
		workspaceState: "initializing",
		...overrides,
	} as WorkspaceStatusFullResponse;
}

function makeMockApi(statuses: WorkspaceStatusFullResponse[]): ApiClient {
	let callIndex = 0;
	return {
		getWorkspaceStatus: vi.fn(async () => {
			const status = statuses[Math.min(callIndex, statuses.length - 1)];
			callIndex++;
			return status;
		}),
	} as unknown as ApiClient;
}

describe("waitForWorkspaceReady", () => {
	it("returns false immediately when environmentError is set", async () => {
		const api = makeMockApi([
			makeStatus({ environmentError: "CrashLoopBackOff: container crashed 3 times" }),
		]);

		const onProgress = vi.fn();
		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 10_000,
			interval: 100,
			onProgress,
		});

		expect(result).toBe(false);
		expect(api.getWorkspaceStatus).toHaveBeenCalledTimes(1);
		expect(onProgress).toHaveBeenCalledWith(
			expect.stringContaining("CrashLoopBackOff"),
		);
	});

	it("returns true when environmentStatus is ready", async () => {
		const api = makeMockApi([
			makeStatus({ environmentStatus: "ready", workspaceReady: true }),
		]);

		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 5_000,
			interval: 100,
		});

		expect(result).toBe(true);
		expect(api.getWorkspaceStatus).toHaveBeenCalledTimes(1);
	});

	it("returns true when workspaceReady is true", async () => {
		const api = makeMockApi([
			makeStatus({ environmentStatus: "initializing", workspaceReady: true }),
		]);

		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 5_000,
			interval: 100,
		});

		expect(result).toBe(true);
	});

	it("polls until ready", async () => {
		const api = makeMockApi([
			makeStatus({ environmentStatus: "initializing" }),
			makeStatus({ environmentStatus: "initializing" }),
			makeStatus({ environmentStatus: "ready", workspaceReady: true }),
		]);

		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 5_000,
			interval: 50,
		});

		expect(result).toBe(true);
		expect(api.getWorkspaceStatus).toHaveBeenCalledTimes(3);
	});

	it("returns false on timeout when never ready", async () => {
		const api = makeMockApi([
			makeStatus({ environmentStatus: "initializing" }),
		]);

		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 200,
			interval: 50,
		});

		expect(result).toBe(false);
	});

	it("reports progress with error message string", async () => {
		const api = makeMockApi([
			makeStatus({ environmentError: "ErrImagePull: image not found" }),
		]);

		const onProgress = vi.fn();
		await waitForWorkspaceReady(api, "task-1", {
			timeout: 5_000,
			interval: 100,
			onProgress,
		});

		expect(onProgress).toHaveBeenCalledWith(
			"Sandbox error: ErrImagePull: image not found",
		);
	});

	it("does not fast-fail when environmentError is null", async () => {
		const api = makeMockApi([
			makeStatus({ environmentStatus: "initializing", environmentError: null }),
			makeStatus({ environmentStatus: "ready", workspaceReady: true }),
		]);

		const result = await waitForWorkspaceReady(api, "task-1", {
			timeout: 5_000,
			interval: 50,
		});

		expect(result).toBe(true);
		expect(api.getWorkspaceStatus).toHaveBeenCalledTimes(2);
	});
});
