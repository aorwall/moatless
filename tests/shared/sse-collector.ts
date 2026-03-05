/**
 * SSE Event Collector for E2E tests.
 *
 * Connects to the NDJSON event stream and collects events in the background.
 * Ported from the Python e2e-test.py implementation.
 */

// =============================================================================
// Types
// =============================================================================

export interface SSEEvent {
	eventId: string;
	type: string;
	subType?: string;
	taskId: string;
	timestamp?: string;
	isError: boolean;
	payload: Record<string, unknown>;
	raw: Record<string, unknown>;
}

export interface TaskLifecycleVerification {
	success: boolean;
	errors: string[];
	receivedEvents: string[];
}

// =============================================================================
// SSE Event Collector
// =============================================================================

export class SSEEventCollector {
	private events: SSEEvent[] = [];
	private abortController: AbortController | null = null;
	private streamPromise: Promise<void> | null = null;
	private error: Error | null = null;
	private debug: boolean;

	constructor(options?: { debug?: boolean }) {
		this.debug = options?.debug ?? false;
	}

	/**
	 * Start collecting SSE events from the given URL
	 */
	async start(url: string): Promise<void> {
		if (this.abortController) {
			this.stop();
		}

		this.abortController = new AbortController();
		this.events = [];
		this.error = null;

		// Start streaming in the background
		this.streamPromise = this.streamEvents(url);

		// Wait a moment for connection to establish
		await new Promise((resolve) => setTimeout(resolve, 1000));

		if (this.error) {
			throw this.error;
		}
	}

	/**
	 * Stop collecting SSE events
	 */
	stop(): void {
		if (this.abortController) {
			this.abortController.abort();
			this.abortController = null;
		}
	}

	/**
	 * Get all collected events
	 */
	getAllEvents(): SSEEvent[] {
		return [...this.events];
	}

	/**
	 * Get all events for a specific task
	 */
	getEventsForTask(taskId: string): SSEEvent[] {
		return this.events.filter((e) => e.taskId === taskId);
	}

	/**
	 * Wait for a specific event to be received
	 */
	async waitForEvent(options: {
		type: string;
		subType?: string;
		taskId?: string;
		timeout?: number;
	}): Promise<SSEEvent | null> {
		const { type, subType, taskId, timeout = 60000 } = options;
		const startTime = Date.now();
		const pollInterval = 500;

		while (Date.now() - startTime < timeout) {
			// Search from most recent events first
			for (let i = this.events.length - 1; i >= 0; i--) {
				const event = this.events[i];
				if (event.type !== type) continue;
				if (subType && event.subType !== subType) continue;
				if (taskId && event.taskId !== taskId) continue;
				return event;
			}

			await new Promise((resolve) => setTimeout(resolve, pollInterval));
		}

		return null;
	}

	/**
	 * Wait for any of multiple event types
	 */
	async waitForAnyEvent(options: {
		events: Array<{ type: string; subType?: string }>;
		taskId?: string;
		timeout?: number;
	}): Promise<SSEEvent | null> {
		const { events: eventPatterns, taskId, timeout = 60000 } = options;
		const startTime = Date.now();
		const pollInterval = 500;

		while (Date.now() - startTime < timeout) {
			for (let i = this.events.length - 1; i >= 0; i--) {
				const event = this.events[i];
				if (taskId && event.taskId !== taskId) continue;

				for (const pattern of eventPatterns) {
					if (event.type !== pattern.type) continue;
					if (pattern.subType && event.subType !== pattern.subType) continue;
					return event;
				}
			}

			await new Promise((resolve) => setTimeout(resolve, pollInterval));
		}

		return null;
	}

	/**
	 * Wait until we've seen N events of a specific type for a task.
	 * Returns the Nth event (1-indexed), or null on timeout.
	 */
	async waitForNthEvent(
		type: string,
		subType: string,
		taskId: string,
		n: number,
		timeoutMs = 300000,
	): Promise<SSEEvent | null> {
		const startTime = Date.now();
		const pollInterval = 500;

		while (Date.now() - startTime < timeoutMs) {
			const matching = this.events.filter(
				(e) => e.type === type && e.subType === subType && e.taskId === taskId,
			);
			if (matching.length >= n) {
				return matching[n - 1];
			}

			await new Promise((resolve) => setTimeout(resolve, pollInterval));
		}

		return null;
	}

	/**
	 * Count events matching a type/subType for a task.
	 */
	countEvents(type: string, subType: string, taskId: string): number {
		return this.events.filter(
			(e) => e.type === type && e.subType === subType && e.taskId === taskId,
		).length;
	}

	/**
	 * Verify that all expected task lifecycle events were received
	 */
	verifyTaskLifecycle(taskId: string): TaskLifecycleVerification {
		const errors: string[] = [];
		const receivedEvents: string[] = [];

		// Required events for full task lifecycle
		const requiredEvents: Array<{
			type: string;
			subType?: string;
			description: string;
		}> = [
			{ type: "task", subType: undefined, description: "Task event" },
			{ type: "environment", subType: "ready", description: "Environment ready" },
			{ type: "server", subType: "started", description: "Server started" },
			{ type: "taskTurn", subType: "active", description: "Iteration active" },
			{ type: "agent", subType: "started", description: "Agent started" },
			{ type: "agent", subType: "completed", description: "Agent completed" },
			{ type: "taskTurn", subType: "completed", description: "Iteration completed" },
		];

		const taskEvents = this.getEventsForTask(taskId);

		for (const required of requiredEvents) {
			const found = taskEvents.some(
				(e) => e.type === required.type && (required.subType === undefined || e.subType === required.subType),
			);

			const eventKey = required.subType ? `${required.type}.${required.subType}` : required.type;

			if (found) {
				receivedEvents.push(eventKey);
			} else {
				errors.push(`Missing: ${required.description} (${eventKey})`);
			}
		}

		// Check for agent messages
		const messages = taskEvents.filter((e) => e.type === "agent" && e.subType === "message");
		if (messages.length > 0) {
			receivedEvents.push(`agent.message (${messages.length})`);
		} else {
			errors.push("Missing: Agent messages (agent.message)");
		}

		return {
			success: errors.length === 0,
			errors,
			receivedEvents,
		};
	}

	/**
	 * Get a summary of events by type
	 */
	getEventSummary(taskId?: string): Record<string, number> {
		const events = taskId ? this.getEventsForTask(taskId) : this.events;
		const summary: Record<string, number> = {};

		for (const event of events) {
			const key = event.subType ? `${event.type}.${event.subType}` : event.type;
			summary[key] = (summary[key] || 0) + 1;
		}

		return summary;
	}

	/**
	 * Check if the collector encountered an error
	 */
	getError(): Error | null {
		return this.error;
	}

	// =========================================================================
	// Private Methods
	// =========================================================================

	private async streamEvents(url: string): Promise<void> {
		try {
			const response = await fetch(url, {
				headers: {
					Accept: "text/event-stream",
				},
				signal: this.abortController?.signal,
			});

			if (!response.ok) {
				throw new Error(`SSE connection failed: ${response.status} ${response.statusText}`);
			}

			if (!response.body) {
				throw new Error("SSE response has no body");
			}

			const reader = response.body.getReader();
			const decoder = new TextDecoder();
			let buffer = "";

			while (true) {
				const { done, value } = await reader.read();

				if (done) break;

				buffer += decoder.decode(value, { stream: true });

				// Process complete lines (NDJSON format)
				const lines = buffer.split("\n");
				buffer = lines.pop() || ""; // Keep incomplete line in buffer

				for (const line of lines) {
					if (!line.trim()) continue;

					try {
						const data = JSON.parse(line);

						// Skip ping events
						if (data.type === "ping") continue;

						const event: SSEEvent = {
							eventId: data.eventId || "",
							type: data.type || "",
							subType: data.subType,
							taskId: data.taskId || "",
							timestamp: data.timestamp,
							isError: data.isError || false,
							payload: data.payload || {},
							raw: data,
						};

						if (this.debug && event.taskId) {
							console.log(`  [SSE] ${event.type}.${event.subType || "*"} for ${event.taskId.slice(0, 8)}`);
						}

						this.events.push(event);
					} catch {
						// Ignore JSON parse errors
					}
				}
			}
		} catch (err) {
			// Only set error if not aborted
			if (err instanceof Error && err.name !== "AbortError") {
				this.error = err;
			}
		}
	}
}
