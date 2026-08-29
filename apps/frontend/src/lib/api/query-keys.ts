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
    subagentTranscript: (taskId: string, toolUseId: string) =>
      [...queryKeys.tasks.detail(taskId), 'subagent-transcript', toolUseId] as const,
    sandboxStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'sandbox'] as const,
    liveStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'live-status'] as const,
    sandboxSetup: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'sandbox-setup'] as const,
    serverStatus: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server'] as const,
    serverLogs: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'server-logs'] as const,

    // File operations - all under 'files' key for targeted invalidation
    files: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'files'] as const,
    fileTree: (taskId: string, mode?: 'shallow' | 'recursive') =>
      [...queryKeys.tasks.files(taskId), 'tree', '.', mode] as const,
    fileContent: (taskId: string, filePath: string | undefined, changes?: boolean) =>
      [...queryKeys.tasks.files(taskId), 'content', filePath, changes] as const,
    events: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'events'] as const,
    connections: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'connections'] as const,
    statistics: (taskId: string) => [...queryKeys.tasks.detail(taskId), 'statistics'] as const,
  },

  auth: {
    all: ['auth'] as const,
    session: () => [...queryKeys.auth.all, 'session'] as const,
    mode: () => [...queryKeys.auth.all, 'mode'] as const,
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
    // The workspace is part of the key because it selects the credential the
    // listing is made with: the same repository can answer differently for a
    // workspace that pins a token and for the viewer alone. Appending it keeps
    // `branches(repoId)` a prefix, so invalidating a repository still clears
    // every workspace's listing of it.
    branches: (repoId: string, workspaceId?: string) =>
      [
        ...queryKeys.repositories.all(),
        'branches',
        repoId,
        ...(workspaceId ? [workspaceId] : []),
      ] as const,
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

  skills: () => [...queryKeys.all, 'skills'] as const,
  // The workspace narrows which plugin skills load, so two workspaces over the
  // same repository are two different lists.
  skillsForRepository: (repositoryId: string, workspaceId?: string) =>
    [...queryKeys.skills(), 'repository', repositoryId, workspaceId ?? null] as const,

  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },

  integrations: {
    all: () => [...queryKeys.all, 'integrations'] as const,
    adapters: () => [...queryKeys.integrations.all(), 'adapters'] as const,
    adapterApps: () => [...queryKeys.integrations.all(), 'adapter-apps'] as const,
    connections: () => [...queryKeys.integrations.all(), 'connections'] as const,
    connectionsList: () => [...queryKeys.integrations.connections(), 'list'] as const,
  },

  loops: {
    all: () => [...queryKeys.all, 'loops'] as const,
    list: () => [...queryKeys.loops.all(), 'list'] as const,
    detail: (id: string) => [...queryKeys.loops.all(), 'detail', id] as const,
    recentTasks: (id: string) => [...queryKeys.loops.detail(id), 'recent-tasks'] as const,
  },

  admin: {
    all: () => [...queryKeys.all, 'admin'] as const,
    taskConnections: (params?: { limit?: number; offset?: number; refresh?: boolean }) =>
      [...queryKeys.admin.all(), 'task-connections', params] as const,
    github: () => [...queryKeys.admin.all(), 'github'] as const,
    githubApps: () => [...queryKeys.admin.github(), 'apps'] as const,
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
    skills: (pluginId: string) => [...queryKeys.plugins.all(), pluginId, 'skills'] as const,
    // Both reads resolve differently per workspace, so the workspace is part of
    // the key rather than something a cached entry can be stale about.
    activations: (pluginId: string, workspaceId?: string) =>
      [...queryKeys.plugins.all(), pluginId, 'activations', workspaceId ?? null] as const,
    effective: (workspaceId?: string) =>
      [...queryKeys.plugins.all(), 'effective', workspaceId ?? null] as const,
  },

  secrets: {
    all: () => [...queryKeys.all, 'secrets'] as const,
    list: (params?: { scope?: string | undefined } | undefined) =>
      [...queryKeys.secrets.all(), 'list', params] as const,
  },

  feedback: {
    all: ['feedback'] as const,
    byTask: (taskId: string) => ['feedback', 'task', taskId] as const,
  },
} as const
