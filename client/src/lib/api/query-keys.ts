export const queryKeys = {
  all: ['swarm'] as const,
  tasks: () => [...queryKeys.all, 'tasks'] as const,
  tasksList: (filters?: { activeOnly?: boolean; limit?: number; offset?: number }) =>
    [...queryKeys.tasks(), 'list', filters] as const,
  // All data for a specific task - when invalidated, all task-related queries are refreshed
  task: (taskId: string) => [...queryKeys.tasks(), taskId] as const,
  taskMessages: (taskId: string, filters?: { limit?: number; offset?: number }) =>
    [...queryKeys.task(taskId), 'messages', filters] as const,
  taskEvents: (taskId: string, filters?: { type?: string }) =>
    [...queryKeys.task(taskId), 'events', filters] as const,
  taskGitDiff: (taskId: string) => [...queryKeys.task(taskId), 'git-diff'] as const,
  taskFileSearch: (taskId: string, query: string) =>
    [...queryKeys.task(taskId), 'file-search', query] as const,
  taskWorkspaceStatus: (taskId: string) => [...queryKeys.task(taskId), 'workspace-status'] as const,
  taskServerStatus: (taskId: string) => [...queryKeys.task(taskId), 'server-status'] as const,
  taskServerLogs: (taskId: string) => [...queryKeys.task(taskId), 'server-logs'] as const,
  taskFileList: (taskId: string, path?: string, recursive?: boolean) =>
    [...queryKeys.task(taskId), 'files', { path, recursive }] as const,
  taskFileContent: (taskId: string, filePath: string) =>
    [...queryKeys.task(taskId), 'file', filePath] as const,
  agents: () => [...queryKeys.all, 'agents'] as const,
  agentsList: () => [...queryKeys.agents(), 'list'] as const,
  environments: () => [...queryKeys.all, 'environments'] as const,
  environmentsList: () => [...queryKeys.environments(), 'list'] as const,
  environmentDetail: (envId: string) => [...queryKeys.environments(), 'detail', envId] as const,
  environmentFileSearch: (envId: string, query: string) =>
    [...queryKeys.environments(), 'file-search', envId, query] as const,
  environmentBranches: (envId: string) => [...queryKeys.environments(), 'branches', envId] as const,
  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },
} as const
