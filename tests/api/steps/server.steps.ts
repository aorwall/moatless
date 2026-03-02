/**
 * Server step functions (SERVER-01 through SERVER-01.2).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type { ServerEntry } from "@moatless/api-client";
import type { ApiClient } from "../../shared/api-client";
import { waitForPreviewReady } from "../../shared/wait-helpers";

// =============================================================================
// SERVER-01: Wait for preview server auto-start
// =============================================================================

export async function waitForPreviewServer(
	api: ApiClient,
	taskId: string,
	timeoutMs = 120000,
): Promise<ServerEntry | null> {
	return waitForPreviewReady(api, taskId, { timeout: timeoutMs });
}

// =============================================================================
// SERVER-01.1: Fetch server URL
// =============================================================================

export async function fetchServerUrl(
	url: string,
	timeoutMs = 10000,
): Promise<{ status: number; body: string }> {
	const response = await fetch(url, {
		signal: AbortSignal.timeout(timeoutMs),
	});
	const body = await response.text();
	return { status: response.status, body };
}
