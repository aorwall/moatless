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
      followedBy?: string
      repositoryId?: string
      parentTaskId?: string
    }) => [...queryKeys.tasks.lists(), filters] as const,

    detail: (taskId: string) => [...queryKeys.tasks.all, 'detail', taskId] as const,

    // Sub-resources - automatically invalidated with parent
    messages: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'messages'] as const,
    lastMessage: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'last-message'] as const,
    toolCall: (taskId: string, toolUseId: string) =>
      [...queryKeys.tasks.detail(taskId), 'tool-call', toolUseId] as const,
    sandboxStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'sandbox'] as const,
    sandboxSetup: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'sandbox-setup'] as const,
    serverStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server'] as const,
    serverLogs: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server-logs'] as const,

    // File operations - all under 'files' key for targeted invalidation
    files: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'files'] as const,
    fileTree: (taskId: string, mode?: 'shallow' | 'recursive') =>
      [...queryKeys.tasks.files(taskId), 'tree', '.', mode] as const,
    fileContent: (taskId: string, filePath: string | undefined, changes?: boolean) =>
      [...queryKeys.tasks.files(taskId), 'content', filePath, changes] as const,
    commands: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'commands'] as const,
    events: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'events'] as const,
    connections: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'connections'] as const,
    statistics: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'statistics'] as const,
  },

  auth: {
    all: ['auth'] as const,
    session: () => [...queryKeys.auth.all, 'session'] as const,
    mode: () => [...queryKeys.auth.all, 'mode'] as const,
  },

  oauth: {
    all: ['oauth'] as const,
    github: () => [...queryKeys.oauth.all, 'github'] as const,
  },

  tags: {
    all: ['tags'] as const,
    list: () => [...queryKeys.tags.all, 'list'] as const,
  },

  users: {
    all: ['users'] as const,
    list: () => [...queryKeys.users.all, 'list'] as const,
    detail: (login: string) => [...queryKeys.users.all, 'detail', login] as const,
  },

  teams: {
    all: ['teams'] as const,
    list: () => [...queryKeys.teams.all, 'list'] as const,
    members: (teamId: string) => [...queryKeys.teams.all, 'members', teamId] as const,
  },

  repositories: {
    all: () => [...queryKeys.all, 'repositories'] as const,
    list: () => [...queryKeys.repositories.all(), 'list'] as const,
    detail: (repoId: string) => [...queryKeys.repositories.all(), 'detail', repoId] as const,
    branches: (repoId: string) => [...queryKeys.repositories.all(), 'branches', repoId] as const,
  },

  workspaces: {
    all: () => [...queryKeys.all, 'workspaces'] as const,
    list: () => [...queryKeys.workspaces.all(), 'list'] as const,
    detail: (workspaceId: string) =>
      [...queryKeys.workspaces.all(), 'detail', workspaceId] as const,
  },

  agents: {
    all: () => [...queryKeys.all, 'agents'] as const,
    list: () => [...queryKeys.agents.all(), 'list'] as const,
  },

  commands: () => [...queryKeys.all, 'commands'] as const,
  commandsList: () => [...queryKeys.commands(), 'list'] as const,
  commandDetail: (commandName: string) => [...queryKeys.commands(), 'detail', commandName] as const,

  skills: () => [...queryKeys.all, 'skills'] as const,
  skillsForRepository: (repositoryId: string) =>
    [...queryKeys.skills(), 'repository', repositoryId] as const,

  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },

  integrations: {
    all: () => [...queryKeys.all, 'integrations'] as const,
    adapters: () => [...queryKeys.integrations.all(), 'adapters'] as const,
    connections: () => [...queryKeys.integrations.all(), 'connections'] as const,
    connectionsList: () => [...queryKeys.integrations.connections(), 'list'] as const,
  },

  triggers: {
    all: () => [...queryKeys.all, 'triggers'] as const,
    list: () => [...queryKeys.triggers.all(), 'list'] as const,
    detail: (id: string) => [...queryKeys.triggers.all(), 'detail', id] as const,
    recentTasks: (id: string) => [...queryKeys.triggers.detail(id), 'recent-tasks'] as const,
  },

  admin: {
    all: () => [...queryKeys.all, 'admin'] as const,
    taskConnections: (params?: { limit?: number; offset?: number; refresh?: boolean }) =>
      [...queryKeys.admin.all(), 'task-connections', params] as const,
  },

  settings: {
    all: () => [...queryKeys.all, 'settings'] as const,
    detail: () => [...queryKeys.settings.all(), 'detail'] as const,
    general: () => [...queryKeys.settings.all(), 'general'] as const,
    codex: () => [...queryKeys.settings.all(), 'codex'] as const,
    codexConfig: () => [...queryKeys.settings.all(), 'codex', 'config'] as const,
    codexDevicePoll: (deviceAuthId: string) =>
      [...queryKeys.settings.all(), 'codex', 'device-poll', deviceAuthId] as const,
    github: () => [...queryKeys.settings.all(), 'github'] as const,
    gitlab: () => [...queryKeys.settings.all(), 'gitlab'] as const,
    gitness: () => [...queryKeys.settings.all(), 'gitness'] as const,
    publish: () => [...queryKeys.settings.all(), 'publish'] as const,
    apiKeys: () => [...queryKeys.settings.all(), 'api-keys'] as const,
  },

  plugins: {
    all: () => [...queryKeys.all, 'plugins'] as const,
    list: () => [...queryKeys.plugins.all(), 'list'] as const,
    marketplaces: () => [...queryKeys.plugins.all(), 'marketplaces'] as const,
    marketplacesList: () => [...queryKeys.plugins.marketplaces(), 'list'] as const,
  },

  secrets: {
    all: () => [...queryKeys.all, 'secrets'] as const,
    list: (params?: { scope: string; scopeId?: string | undefined } | undefined) =>
      [...queryKeys.secrets.all(), 'list', params] as const,
  },

  feedback: {
    all: ['feedback'] as const,
    byTask: (taskId: string) => ['feedback', 'task', taskId] as const,
  },

  taskTemplates: {
    all: () => [...queryKeys.all, 'task-templates'] as const,
    list: () => [...queryKeys.taskTemplates.all(), 'list'] as const,
    featured: () => [...queryKeys.taskTemplates.all(), 'featured'] as const,
    detail: (id: string) => [...queryKeys.taskTemplates.all(), 'detail', id] as const,
  },
} as const
