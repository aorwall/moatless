export const queryKeys = {
  all: ['swarm'] as const,

  // New hierarchical task structure
  tasks: {
    all: ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all, 'list'] as const,
    list: (filters?: { activeOnly?: boolean; limit?: number; offset?: number; status?: string }) =>
      [...queryKeys.tasks.lists(), filters] as const,

    details: () => [...queryKeys.tasks.all, 'detail'] as const,
    detail: (id: string) => [...queryKeys.tasks.details(), id] as const,

    // Sub-resources - automatically invalidated with parent
    events: (id: string) => [...queryKeys.tasks.detail(id), 'events'] as const,
    messages: (id: string, filters?: { limit?: number; offset?: number }) =>
      [...queryKeys.tasks.detail(id), 'messages', filters] as const,
    iterations: (id: string) => [...queryKeys.tasks.detail(id), 'iterations'] as const,
    gitDiff: (id: string) => [...queryKeys.tasks.detail(id), 'git-diff'] as const,
    workspaceStatus: (id: string) => [...queryKeys.tasks.detail(id), 'workspace'] as const,
    serverStatus: (id: string) => [...queryKeys.tasks.detail(id), 'server'] as const,
    serverLogs: (id: string) => [...queryKeys.tasks.detail(id), 'server-logs'] as const,
    fileSearch: (id: string, query: string) =>
      [...queryKeys.tasks.detail(id), 'file-search', query] as const,
    fileList: (id: string, path?: string, recursive?: boolean) =>
      [...queryKeys.tasks.detail(id), 'files', { path, recursive }] as const,
    fileContent: (id: string, filePath: string) =>
      [...queryKeys.tasks.detail(id), 'file', filePath] as const,
  },

  // Legacy keys for backward compatibility during migration
  tasksList: (filters?: { activeOnly?: boolean; limit?: number; offset?: number }) =>
    queryKeys.tasks.list(filters),
  task: (taskId: string) => queryKeys.tasks.detail(taskId),
  taskMessages: (taskId: string, filters?: { limit?: number; offset?: number }) =>
    queryKeys.tasks.messages(taskId, filters),
  taskEvents: (taskId: string, filters?: { type?: string }) =>
    [...queryKeys.tasks.detail(taskId), 'events', filters] as const,
  taskGitDiff: (taskId: string) => queryKeys.tasks.gitDiff(taskId),
  taskFileSearch: (taskId: string, query: string) => queryKeys.tasks.fileSearch(taskId, query),
  taskWorkspaceStatus: (taskId: string) => queryKeys.tasks.workspaceStatus(taskId),
  taskServerStatus: (taskId: string) => queryKeys.tasks.serverStatus(taskId),
  taskServerLogs: (taskId: string) => queryKeys.tasks.serverLogs(taskId),
  taskFileList: (taskId: string, path?: string, recursive?: boolean) =>
    queryKeys.tasks.fileList(taskId, path, recursive),
  taskFileContent: (taskId: string, filePath: string) =>
    queryKeys.tasks.fileContent(taskId, filePath),

  agents: () => [...queryKeys.all, 'agents'] as const,
  agentsList: () => [...queryKeys.agents(), 'list'] as const,
  repositories: () => [...queryKeys.all, 'repositories'] as const,
  repositoriesList: () => [...queryKeys.repositories(), 'list'] as const,
  repositoryDetail: (repoId: string) => [...queryKeys.repositories(), 'detail', repoId] as const,
  repositoryFileSearch: (repoId: string, query: string) =>
    [...queryKeys.repositories(), 'file-search', repoId, query] as const,
  repositoryBranches: (repoId: string) =>
    [...queryKeys.repositories(), 'branches', repoId] as const,
  // Legacy environment keys for backward compatibility
  environments: () => queryKeys.repositories(),
  environmentsList: () => queryKeys.repositoriesList(),
  environmentDetail: (envId: string) => queryKeys.repositoryDetail(envId),
  environmentFileSearch: (envId: string, query: string) =>
    queryKeys.repositoryFileSearch(envId, query),
  environmentBranches: (envId: string) => queryKeys.repositoryBranches(envId),
  commands: () => [...queryKeys.all, 'commands'] as const,
  commandsList: () => [...queryKeys.commands(), 'list'] as const,
  commandDetail: (commandName: string) => [...queryKeys.commands(), 'detail', commandName] as const,
  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },
  templates: {
    all: () => [...queryKeys.all, 'templates'] as const,
    list: () => [...queryKeys.templates.all(), 'list'] as const,
  },
} as const
