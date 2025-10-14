import type { Event, RepositoryConfig, Task, Workspace } from '@moatless/types'
import { vi } from 'vitest'
import type { EventService } from '../../services/event.service.js'
import type { StorageBackend } from '../../storage/index.js'

// Storage Backend Mocks
export function createMockStorageBackend(overrides?: Partial<StorageBackend>): StorageBackend {
  return {
    save: vi.fn().mockResolvedValue(undefined),
    get: vi.fn().mockResolvedValue(null),
    list: vi.fn().mockResolvedValue([]),
    delete: vi.fn().mockResolvedValue(undefined),
    exists: vi.fn().mockResolvedValue(false),
    appendJsonl: vi.fn().mockResolvedValue(undefined),
    readJsonl: vi.fn().mockResolvedValue([]),
    saveTask: vi.fn().mockResolvedValue(undefined),
    getTask: vi.fn().mockResolvedValue(null),
    saveRepository: vi.fn().mockResolvedValue(undefined),
    getRepository: vi.fn().mockResolvedValue(null),
    saveWorkspace: vi.fn().mockResolvedValue(undefined),
    getWorkspace: vi.fn().mockResolvedValue(null),
    saveEvents: vi.fn().mockResolvedValue(undefined),
    getEvents: vi.fn().mockResolvedValue([]),
    saveUser: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    listUsers: vi.fn().mockResolvedValue([]),
    saveFigmaImport: vi.fn().mockResolvedValue(undefined),
    getFigmaImport: vi.fn().mockResolvedValue(null),
    ...overrides,
  }
}

export function createMockStorageWithData(
  tasks: Task[] = [],
  repositories: RepositoryConfig[] = [],
  workspaces: Workspace[] = [],
  events: Record<string, Event[]> = {},
): StorageBackend {
  const storage = createMockStorageBackend()

  // Set up get methods to return actual data
  vi.mocked(storage.getTask).mockImplementation(async (taskId: string) => {
    return tasks.find((t) => t.taskId === taskId) || null
  })

  vi.mocked(storage.getRepository).mockImplementation(async (repoId: string) => {
    return repositories.find((r) => r.id === repoId) || null
  })

  vi.mocked(storage.getWorkspace).mockImplementation(async (taskId: string) => {
    return workspaces.find((w) => w.id === taskId) || null
  })

  vi.mocked(storage.getEvents).mockImplementation(async (taskId: string) => {
    return events[taskId] || []
  })

  // Set up exists method
  vi.mocked(storage.exists).mockImplementation(async (key: string) => {
    if (key.startsWith('tasks/')) {
      const taskId = key.replace('tasks/', '')
      return tasks.some((t) => t.taskId === taskId)
    }
    if (key.startsWith('repositories/')) {
      const repoId = key.replace('repositories/', '')
      return repositories.some((r) => r.id === repoId)
    }
    if (key.startsWith('workspaces/')) {
      const workspaceId = key.replace('workspaces/', '')
      return workspaces.some((w) => w.id === workspaceId)
    }
    return false
  })

  return storage
}

// Event Service Mocks
export function createMockEventService(overrides?: Partial<EventService>): EventService {
  const mockEventService = {
    emitEvent: vi.fn().mockResolvedValue(undefined),
    getEvents: vi.fn().mockResolvedValue([]),
    subscribe: vi.fn().mockReturnValue(() => undefined),
    subscribeAll: vi.fn().mockReturnValue(() => undefined),
    getEventCount: vi.fn().mockResolvedValue(0),
    // EventEmitter methods
    on: vi.fn(),
    off: vi.fn(),
    emit: vi.fn(),
    removeAllListeners: vi.fn(),
    setMaxListeners: vi.fn(),
    listeners: vi.fn().mockReturnValue([]),
    listenerCount: vi.fn().mockReturnValue(0),
    ...overrides,
  } as unknown as EventService

  return mockEventService
}

export function createMockEventServiceWithEvents(events: Event[]): EventService {
  const eventService = createMockEventService()

  vi.mocked(eventService.getEvents).mockImplementation(
    async (taskId: string, limit?: number, offset = 0) => {
      const taskEvents = events.filter((e) => e.taskId === taskId)
      return taskEvents.slice(offset, limit ? offset + limit : undefined)
    },
  )

  vi.mocked(eventService.getEventCount).mockImplementation(async (taskId: string) => {
    return events.filter((e) => e.taskId === taskId).length
  })

  return eventService
}

// Logger Mocks
interface Logger {
  trace(message: string, meta?: Record<string, unknown>): void
  debug(message: string, meta?: Record<string, unknown>): void
  info(message: string, meta?: Record<string, unknown>): void
  warn(message: string, meta?: Record<string, unknown>): void
  error(message: string, error?: Error | unknown, meta?: Record<string, unknown>): void
  fatal(message: string, error?: Error | unknown, meta?: Record<string, unknown>): void
  child(bindings: Record<string, unknown>): Logger
}

export function createMockLogger(overrides?: Partial<Logger>): Logger {
  const mockLogger = {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    child: vi.fn().mockReturnValue(createMockLogger()),
    ...overrides,
  }

  return mockLogger
}

// Settings Service Mock
interface ClaudeSettings {
  binDir?: string
  configDir?: string
  oauthToken?: string
}

interface GithubSettings {
  personalAccessToken?: string
  userName?: string
  userEmail?: string
}

interface GeneralSettings {
  dataDir?: string
  repositoriesDir?: string
  defaultWorkspaceType?: 'local' | 'docker'
}

interface SettingsService {
  getClaudeSettings(): Promise<ClaudeSettings>
  updateClaudeSettings(settings: ClaudeSettings): Promise<void>
  getGithubSettings(): Promise<GithubSettings>
  updateGithubSettings(settings: GithubSettings): Promise<void>
  getGeneralSettings(): Promise<GeneralSettings>
  updateGeneralSettings(settings: GeneralSettings): Promise<void>
}

export function createMockSettingsService(overrides?: Partial<SettingsService>): SettingsService {
  return {
    getClaudeSettings: vi.fn().mockResolvedValue({}),
    updateClaudeSettings: vi.fn().mockResolvedValue(undefined),
    getGithubSettings: vi.fn().mockResolvedValue({}),
    updateGithubSettings: vi.fn().mockResolvedValue(undefined),
    getGeneralSettings: vi.fn().mockResolvedValue({}),
    updateGeneralSettings: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
}

// Agent Service Mock
interface AgentInstance {
  agentId: string
  taskId: string
  status: 'running' | 'stopped'
}

interface AgentService {
  createAgent(taskId: string, agentType: string): Promise<AgentInstance>
  stopAgent(taskId: string): Promise<void>
  getAgent(taskId: string): AgentInstance | null
  isAgentRunning(taskId: string): boolean
}

export function createMockAgentService(overrides?: Partial<AgentService>): AgentService {
  return {
    createAgent: vi.fn().mockResolvedValue({
      agentId: 'mock-agent',
      taskId: 'mock-task',
      status: 'running',
    }),
    stopAgent: vi.fn().mockResolvedValue(undefined),
    getAgent: vi.fn().mockReturnValue(null),
    isAgentRunning: vi.fn().mockReturnValue(false),
    ...overrides,
  }
}

// Task Service Mock
interface CreateTaskRequest {
  repositoryId: string
  message?: string
  commandName?: string
  branchName?: string
}

interface TaskFilters {
  repositoryId?: string
  status?: string
  limit?: number
  offset?: number
}

interface TaskService {
  createTask(request: CreateTaskRequest): Promise<Task>
  getTask(taskId: string): Promise<Task | null>
  updateTask(taskId: string, updates: Partial<Task>): Promise<void>
  closeTask(taskId: string): Promise<void>
  getTasks(filters?: TaskFilters): Promise<Task[]>
}

export function createMockTaskService(overrides?: Partial<TaskService>): TaskService {
  return {
    createTask: vi.fn().mockResolvedValue({} as Task),
    getTask: vi.fn().mockResolvedValue(null),
    updateTask: vi.fn().mockResolvedValue(undefined),
    closeTask: vi.fn().mockResolvedValue(undefined),
    getTasks: vi.fn().mockResolvedValue([]),
    ...overrides,
  }
}

// Workspace Service Mock
interface WorkspaceStatus {
  status: 'idle' | 'setting_up' | 'ready' | 'error'
  environmentStatus?: string
  error?: string
}

interface WorkspaceService {
  createWorkspace(taskId: string, repositoryId: string): Promise<Workspace>
  getWorkspace(taskId: string): Promise<Workspace | null>
  setupWorkspace(taskId: string): Promise<void>
  cleanupWorkspace(taskId: string): Promise<void>
  getWorkspaceStatus(taskId: string): Promise<WorkspaceStatus>
}

export function createMockWorkspaceService(
  overrides?: Partial<WorkspaceService>,
): WorkspaceService {
  return {
    createWorkspace: vi.fn().mockResolvedValue({} as Workspace),
    getWorkspace: vi.fn().mockResolvedValue(null),
    setupWorkspace: vi.fn().mockResolvedValue(undefined),
    cleanupWorkspace: vi.fn().mockResolvedValue(undefined),
    getWorkspaceStatus: vi.fn().mockResolvedValue({ status: 'ready' }),
    ...overrides,
  }
}
