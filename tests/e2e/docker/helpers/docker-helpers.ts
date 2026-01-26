/**
 * Docker-specific helpers for E2E tests.
 *
 * Provides utilities for container inspection, logs, and network verification.
 */

import { exec } from "node:child_process";
import { promises as fs } from "node:fs";
import { promisify } from "node:util";

const execAsync = promisify(exec);

// =============================================================================
// Types
// =============================================================================

export interface ContainerState {
	status: string;
	exitCode: number;
	running: boolean;
	error?: string;
}

export interface ContainerNetwork {
	name: string;
	ipAddress?: string;
}

// =============================================================================
// Docker Availability
// =============================================================================

/**
 * Check if Docker daemon is running and accessible
 */
export async function isDockerAvailable(): Promise<boolean> {
	try {
		const { stdout } = await execAsync("docker info --format '{{.ServerVersion}}'", {
			timeout: 5000,
		});
		return stdout.trim().length > 0;
	} catch {
		return false;
	}
}

/**
 * Check if a specific Docker image exists locally
 */
export async function imageExists(imageName: string): Promise<boolean> {
	try {
		const { stdout } = await execAsync(`docker images -q ${imageName}`, {
			timeout: 5000,
		});
		return stdout.trim().length > 0;
	} catch {
		return false;
	}
}

// =============================================================================
// Container Operations
// =============================================================================

/**
 * Get the state of a container
 */
export async function getContainerState(containerName: string): Promise<ContainerState | null> {
	try {
		const { stdout } = await execAsync(
			`docker inspect --format='{{.State.Status}} {{.State.ExitCode}} {{.State.Running}}' ${containerName}`,
			{ timeout: 10000 },
		);

		const parts = stdout.trim().split(" ");
		if (parts.length >= 3) {
			return {
				status: parts[0],
				exitCode: Number.parseInt(parts[1], 10),
				running: parts[2] === "true",
			};
		}

		return null;
	} catch (err) {
		return {
			status: "not_found",
			exitCode: -1,
			running: false,
			error: err instanceof Error ? err.message : String(err),
		};
	}
}

/**
 * Get container logs
 *
 * @param containerName - Name or ID of the container
 * @param tail - Number of lines to return (undefined for all)
 * @returns Container logs as a string
 */
export async function getContainerLogs(containerName: string, tail?: number): Promise<string> {
	try {
		const tailArg = tail ? `--tail ${tail}` : "";
		const { stdout, stderr } = await execAsync(`docker logs ${tailArg} ${containerName}`, {
			maxBuffer: 10 * 1024 * 1024, // 10MB buffer for large logs
			timeout: 30000,
		});

		return `=== STDOUT ===\n${stdout}\n\n=== STDERR ===\n${stderr}`;
	} catch (err) {
		return `Error getting logs: ${err instanceof Error ? err.message : String(err)}`;
	}
}

/**
 * Save container logs to a file
 */
export async function saveContainerLogs(
	containerName: string,
	outputPath: string,
	options?: { tail?: number; testName?: string },
): Promise<void> {
	const logs = await getContainerLogs(containerName, options?.tail);

	const header = `=== CONTAINER LOGS ===
Container: ${containerName}
Captured at: ${new Date().toISOString()}
Test: ${options?.testName || "unknown"}

`;

	await fs.writeFile(outputPath, header + logs, "utf-8");
}

/**
 * Get the networks a container is connected to
 */
export async function getContainerNetworks(containerName: string): Promise<ContainerNetwork[]> {
	try {
		const { stdout } = await execAsync(
			`docker inspect --format='{{range $k, $v := .NetworkSettings.Networks}}{{$k}}:{{$v.IPAddress}},{{end}}' ${containerName}`,
			{ timeout: 10000 },
		);

		const networks: ContainerNetwork[] = [];
		const parts = stdout.trim().split(",").filter(Boolean);

		for (const part of parts) {
			const [name, ipAddress] = part.split(":");
			if (name) {
				networks.push({ name, ipAddress: ipAddress || undefined });
			}
		}

		return networks;
	} catch {
		return [];
	}
}

/**
 * Verify container is on the expected network
 */
export async function verifyContainerNetwork(
	containerName: string,
	expectedNetwork: string,
): Promise<boolean> {
	const networks = await getContainerNetworks(containerName);
	return networks.some((n) => n.name === expectedNetwork);
}

/**
 * Get the port mapping for a container
 *
 * @param containerName - Name or ID of the container
 * @param containerPort - Port inside the container (e.g., 3000)
 * @returns Host port number or null if not mapped
 */
export async function getContainerPort(
	containerName: string,
	containerPort: number,
): Promise<number | null> {
	try {
		const { stdout } = await execAsync(`docker port ${containerName} ${containerPort}`, {
			timeout: 5000,
		});

		// Format: 0.0.0.0:PORT or [::]:PORT
		const portStr = stdout.trim().split(":").pop();
		if (portStr) {
			return Number.parseInt(portStr, 10);
		}
		return null;
	} catch {
		return null;
	}
}

// =============================================================================
// Container Name Helpers
// =============================================================================

/**
 * Get the container name for a task
 *
 * Convention: moatless-moatable-{taskId-prefix}
 */
export function getContainerNameForTask(taskId: string): string {
	return `moatless-moatable-${taskId.slice(0, 8)}`;
}

/**
 * List all moatless containers
 */
export async function listMoatlessContainers(): Promise<string[]> {
	try {
		const { stdout } = await execAsync(
			'docker ps -a --format "{{.Names}}" --filter "name=moatless-"',
			{ timeout: 10000 },
		);

		return stdout
			.trim()
			.split("\n")
			.filter((name) => name.length > 0);
	} catch {
		return [];
	}
}

// =============================================================================
// Cleanup Helpers
// =============================================================================

/**
 * Remove a container (force)
 */
export async function removeContainer(containerName: string): Promise<boolean> {
	try {
		await execAsync(`docker rm -f ${containerName}`, { timeout: 30000 });
		return true;
	} catch {
		return false;
	}
}

/**
 * Clean up all moatless test containers
 */
export async function cleanupTestContainers(): Promise<number> {
	const containers = await listMoatlessContainers();
	let removed = 0;

	for (const container of containers) {
		if (await removeContainer(container)) {
			removed++;
		}
	}

	return removed;
}

// =============================================================================
// Health Check
// =============================================================================

/**
 * Check if the docker-compose environment is healthy
 */
export async function isDockerComposeHealthy(backendUrl = "http://localhost:8000"): Promise<{
	healthy: boolean;
	dockerAvailable: boolean;
	backendHealthy: boolean;
	error?: string;
}> {
	const dockerAvailable = await isDockerAvailable();
	if (!dockerAvailable) {
		return {
			healthy: false,
			dockerAvailable: false,
			backendHealthy: false,
			error: "Docker daemon is not running",
		};
	}

	// Check backend health
	try {
		const response = await fetch(`${backendUrl}/health`, {
			signal: AbortSignal.timeout(5000),
		});
		const data = await response.json();
		const backendHealthy = data.status === "ok";

		return {
			healthy: backendHealthy,
			dockerAvailable: true,
			backendHealthy,
			error: backendHealthy ? undefined : "Backend is not healthy",
		};
	} catch (err) {
		return {
			healthy: false,
			dockerAvailable: true,
			backendHealthy: false,
			error: `Backend connection failed: ${err instanceof Error ? err.message : String(err)}`,
		};
	}
}
