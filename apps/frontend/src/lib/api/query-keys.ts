export const queryKeys = {
  all: ['swarm'] as const,

  // New hierarchical task structure
  tasks: {
    all: ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all, 'list'] as const,
    list: (filters?: { activeOnly?: boolean; limit?: number; offset?: number; status?: string }) =>
      [...queryKeys.tasks.lists(), filters] as const,

    detail: (id: string) => [...queryKeys.tasks.all, 'detail', id] as const,

    // Sub-resources - automatically invalidated with parent
    messages: (id: string, filters?: { limit?: number; offset?: number }) =>
      [...queryKeys.tasks.detail(id), 'messages', filters] as const,
    gitDiff: (id: string) => [...queryKeys.tasks.detail(id), 'git-diff'] as const,
    workspaceStatus: (id: string) => [...queryKeys.tasks.detail(id), 'workspace'] as const,
    workspaceSetup: (id: string) => [...queryKeys.tasks.detail(id), 'workspace-setup'] as const,
    serverStatus: (id: string) => [...queryKeys.tasks.detail(id), 'server'] as const,
    serverLogs: (id: string) => [...queryKeys.tasks.detail(id), 'server-logs'] as const,
    fileList: (id: string, path?: string, recursive?: boolean) =>
      [...queryKeys.tasks.detail(id), 'files', { path, recursive }] as const,
    fileContent: (id: string, filePath: string) =>
      [...queryKeys.tasks.detail(id), 'file', filePath] as const,
    commands: (id: string) => [...queryKeys.tasks.detail(id), 'commands'] as const,
  },

  repositories: () => [...queryKeys.all, 'repositories'] as const,
  repositoriesList: () => [...queryKeys.repositories(), 'list'] as const,
  repositoryDetail: (repoId: string) => [...queryKeys.repositories(), 'detail', repoId] as const,
  repositoryBranches: (repoId: string) =>
    [...queryKeys.repositories(), 'branches', repoId] as const,

  commands: () => [...queryKeys.all, 'commands'] as const,
  commandsList: () => [...queryKeys.commands(), 'list'] as const,
  commandDetail: (commandName: string) => [...queryKeys.commands(), 'detail', commandName] as const,

  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },

  settings: {
    all: () => [...queryKeys.all, 'settings'] as const,
    detail: () => [...queryKeys.settings.all(), 'detail'] as const,
    systemStatus: () => [...queryKeys.settings.all(), 'system-status'] as const,
    general: () => [...queryKeys.settings.all(), 'general'] as const,
    claude: () => [...queryKeys.settings.all(), 'claude'] as const,
    github: () => [...queryKeys.settings.all(), 'github'] as const,
    mcp: () => [...queryKeys.settings.all(), 'mcp'] as const,
    publish: () => [...queryKeys.settings.all(), 'publish'] as const,
  },
  templates: {
    all: () => [...queryKeys.all, 'templates'] as const,
    list: () => [...queryKeys.templates.all(), 'list'] as const,
  },
} as const
