export const queryKeys = {
  all: ['swarm'] as const,

  // New hierarchical task structure
  tasks: {
    all: ['tasks'] as const,
    lists: () => [...queryKeys.tasks.all, 'list'] as const,
    list: (filters?: {
      activeOnly?: boolean
      limit?: number
      offset?: number
      status?: string
      tag?: string
      createdBy?: string
      repositoryId?: string
    }) => [...queryKeys.tasks.lists(), filters] as const,

    detail: (taskId: string) => [...queryKeys.tasks.all, 'detail', taskId] as const,

    // Sub-resources - automatically invalidated with parent
    messages: (taskId: string, filters?: { limit?: number; offset?: number }) =>
      [...queryKeys.tasks.detail(taskId), 'messages', filters] as const,
    workspaceStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'workspace'] as const,
    workspaceSetup: (taskId: string) =>
      [...queryKeys.tasks.detail(taskId), 'workspace-setup'] as const,
    serverStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server'] as const,
    serverLogs: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server-logs'] as const,
    servers: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'servers'] as const,

    // File operations - all under 'files' key for targeted invalidation
    files: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'files'] as const,
    commands: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'commands'] as const,
  },

  tags: {
    all: ['tags'] as const,
    list: () => [...queryKeys.tags.all, 'list'] as const,
  },

  users: {
    all: ['users'] as const,
    list: () => [...queryKeys.users.all, 'list'] as const,
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
    general: () => [...queryKeys.settings.all(), 'general'] as const,
    claude: () => [...queryKeys.settings.all(), 'claude'] as const,
    claudeConfig: () => [...queryKeys.settings.all(), 'claude', 'config'] as const,
    claudeOAuthUrl: () => [...queryKeys.settings.all(), 'claude', 'oauth-url'] as const,
    github: () => [...queryKeys.settings.all(), 'github'] as const,
    gitlab: () => [...queryKeys.settings.all(), 'gitlab'] as const,
    publish: () => [...queryKeys.settings.all(), 'publish'] as const,
    figma: () => [...queryKeys.settings.all(), 'figma'] as const,
    apiKeys: () => [...queryKeys.settings.all(), 'api-keys'] as const,
  },

  plugins: {
    all: () => [...queryKeys.all, 'plugins'] as const,
    list: () => [...queryKeys.plugins.all(), 'list'] as const,
    marketplaces: () => [...queryKeys.plugins.all(), 'marketplaces'] as const,
    marketplacesList: () => [...queryKeys.plugins.marketplaces(), 'list'] as const,
  },
} as const
