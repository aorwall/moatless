export const queryKeys = {
  all: ['swarm'] as const,
  sessions: () => [...queryKeys.all, 'sessions'] as const,
  sessionsList: (filters?: {
    activeOnly?: boolean;
    limit?: number;
    offset?: number;
  }) => [...queryKeys.sessions(), 'list', filters] as const,
  sessionDetail: (sessionId: string) =>
    [...queryKeys.sessions(), 'detail', sessionId] as const,
  sessionMessages: (
    sessionId: string,
    filters?: { limit?: number; offset?: number },
  ) => [...queryKeys.sessions(), 'messages', sessionId, filters] as const,
  sessionEvents: (
    sessionId: string,
    filters?: { limit?: number; offset?: number },
  ) => [...queryKeys.sessions(), 'events', sessionId, filters] as const,
  sessionGitDiff: (sessionId: string) =>
    [...queryKeys.sessions(), 'git-diff', sessionId] as const,
  agents: () => [...queryKeys.all, 'agents'] as const,
  agentsList: () => [...queryKeys.agents(), 'list'] as const,
  environments: () => [...queryKeys.all, 'environments'] as const,
  environmentsList: () => [...queryKeys.environments(), 'list'] as const,
  environmentDetail: (envId: string) =>
    [...queryKeys.environments(), 'detail', envId] as const,
  environmentFileSearch: (envId: string, query: string) =>
    [...queryKeys.environments(), 'file-search', envId, query] as const,
  claudeCode: {
    all: () => [...queryKeys.all, 'claude-code'] as const,
    settings: () => [...queryKeys.claudeCode.all(), 'settings'] as const,
    tools: () => [...queryKeys.claudeCode.all(), 'tools'] as const,
  },
} as const;
