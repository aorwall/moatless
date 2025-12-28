import client from 'prom-client'

// Create a Registry
const register = new client.Registry()

// Add default metrics (heap, RSS, CPU, event loop lag, GC, etc.)
// These include:
// - process_cpu_user_seconds_total
// - process_cpu_system_seconds_total
// - process_resident_memory_bytes (RSS - what we need for OOM debugging)
// - process_heap_bytes
// - nodejs_heap_size_total_bytes
// - nodejs_heap_size_used_bytes
// - nodejs_external_memory_bytes
// - nodejs_eventloop_lag_seconds
// - nodejs_gc_duration_seconds
// - nodejs_active_handles_total
// - nodejs_active_requests_total
client.collectDefaultMetrics({
  register,
  prefix: 'moatless_',
})

// Custom metrics for application-specific tracking
export const taskExecutionCounter = new client.Counter({
  name: 'moatless_task_executions_total',
  help: 'Total number of task executions',
  labelNames: ['status', 'agent_type'],
  registers: [register],
})

export const taskDurationHistogram = new client.Histogram({
  name: 'moatless_task_duration_seconds',
  help: 'Task execution duration in seconds',
  labelNames: ['agent_type'],
  buckets: [1, 5, 10, 30, 60, 120, 300, 600],
  registers: [register],
})

export const activeTasksGauge = new client.Gauge({
  name: 'moatless_active_tasks',
  help: 'Number of currently active tasks',
  registers: [register],
})

export const activeWorkspacesGauge = new client.Gauge({
  name: 'moatless_active_workspaces',
  help: 'Number of currently active workspaces',
  registers: [register],
})

export const sseConnectionsGauge = new client.Gauge({
  name: 'moatless_sse_connections',
  help: 'Number of active SSE connections',
  registers: [register],
})

export { register }
