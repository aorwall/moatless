import {
  IntegrationProvider,
  type ProviderConnectionResponse,
  RepositoryKind,
  type RepositoryResponse,
  RoutingMode,
  Scope,
  type TagResponse,
  type TaskRouteResponse,
  type TaskTriggerResponse,
  Visibility,
} from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { IntegrationRoutingSection } from '../integration-routing-section'

const mockUseIntegrationProviders = vi.hoisted(() => vi.fn())
const mockUseProviderConnections = vi.hoisted(() => vi.fn())
const mockUseTaskRoutes = vi.hoisted(() => vi.fn())
const mockUseTaskTriggers = vi.hoisted(() => vi.fn())
const mockUseUpdateTaskTrigger = vi.hoisted(() => vi.fn())
const mockUseDeleteTaskTrigger = vi.hoisted(() => vi.fn())
const mockUseCreateProviderConnection = vi.hoisted(() => vi.fn())
const mockUseCreateTaskRoute = vi.hoisted(() => vi.fn())
const mockUseUpdateTaskRoute = vi.hoisted(() => vi.fn())
const mockUseCreateTaskTrigger = vi.hoisted(() => vi.fn())
const mockUseRepositoriesList = vi.hoisted(() => vi.fn())
const mockUseTagsList = vi.hoisted(() => vi.fn())

vi.mock('@/features/settings/hooks/use-integrations', () => ({
  useIntegrationProviders: mockUseIntegrationProviders,
  useProviderConnections: mockUseProviderConnections,
  useTaskRoutes: mockUseTaskRoutes,
  useTaskTriggers: mockUseTaskTriggers,
  useUpdateTaskTrigger: mockUseUpdateTaskTrigger,
  useDeleteTaskTrigger: mockUseDeleteTaskTrigger,
  useCreateProviderConnection: mockUseCreateProviderConnection,
  useCreateTaskRoute: mockUseCreateTaskRoute,
  useUpdateTaskRoute: mockUseUpdateTaskRoute,
  useCreateTaskTrigger: mockUseCreateTaskTrigger,
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: mockUseRepositoriesList,
}))

vi.mock('@/features/tags', () => ({
  useTagsList: mockUseTagsList,
}))

function successQuery<TData>(data: TData) {
  return { data, isLoading: false, error: null }
}

function idleMutation() {
  return {
    mutate: vi.fn(),
    mutateAsync: vi.fn(),
    isPending: false,
  }
}

const repository: RepositoryResponse = {
  id: 'repo-1',
  name: 'Repo One',
  kind: RepositoryKind.git,
  scope: Scope.user,
  visibility: Visibility.private,
}

const tags: TagResponse[] = [
  { tagId: 'tag-1', name: 'Urgent' },
  { tagId: 'tag-2', name: 'Support' },
]

const route: TaskRouteResponse = {
  id: 'route-1',
  repositoryId: repository.id,
  routingMode: RoutingMode.thread_based,
  promptTemplate: 'Use {message}',
  taskName: 'Bug bash',
  tagIds: tags.map((tag) => tag.tagId),
  agentType: 'claude',
  createdBy: 'user-1',
  createdAt: '2026-05-07T10:00:00Z',
  updatedAt: '2026-05-07T11:00:00Z',
  scope: Scope.user,
}

const connection: ProviderConnectionResponse = {
  id: 'connection-1',
  provider: IntegrationProvider.slack,
  externalAccountId: 'T123',
  connectionKind: 'bot_api',
  createdBy: 'user-1',
  createdAt: '2026-05-07T10:00:00Z',
  updatedAt: '2026-05-07T10:00:00Z',
  scope: Scope.user,
}

const trigger: TaskTriggerResponse = {
  id: 'trigger-1',
  provider: IntegrationProvider.slack,
  connectionId: connection.id,
  sourceMatcher: 'C123',
  sourceName: '#triage',
  routeId: route.id,
  triggerConfig: { trigger: 'mention_only' },
  eventFilter: {},
  active: true,
  allowDirectSend: false,
  closeTaskOnClose: true,
  notifyOnSuccess: false,
  createdBy: 'user-1',
  createdAt: '2026-05-07T10:00:00Z',
  updatedAt: '2026-05-07T11:00:00Z',
  scope: Scope.user,
}

describe('IntegrationRoutingSection', () => {
  beforeEach(() => {
    mockUseIntegrationProviders.mockReturnValue(successQuery([IntegrationProvider.slack]))
    mockUseProviderConnections.mockReturnValue(successQuery([connection]))
    mockUseTaskRoutes.mockReturnValue(successQuery([route]))
    mockUseTaskTriggers.mockReturnValue(successQuery([trigger]))
    mockUseRepositoriesList.mockReturnValue(successQuery([repository]))
    mockUseTagsList.mockReturnValue(successQuery(tags))
    mockUseUpdateTaskTrigger.mockReturnValue(idleMutation())
    mockUseDeleteTaskTrigger.mockReturnValue(idleMutation())
    mockUseCreateProviderConnection.mockReturnValue(idleMutation())
    mockUseCreateTaskRoute.mockReturnValue(idleMutation())
    mockUseUpdateTaskRoute.mockReturnValue(idleMutation())
    mockUseCreateTaskTrigger.mockReturnValue(idleMutation())
  })

  it('renders task route rows from canonical route data', () => {
    render(<IntegrationRoutingSection />)

    expect(screen.getByText('Repo One')).toBeInTheDocument()
    expect(screen.getByText('Thread Based')).toBeInTheDocument()
    expect(screen.getByText('Bug bash')).toBeInTheDocument()
    expect(screen.getByText('Configured')).toBeInTheDocument()
  })

  it('renders trigger rows joined with routes and connections', async () => {
    const user = userEvent.setup()
    render(<IntegrationRoutingSection />)

    await user.click(screen.getByRole('tab', { name: 'Triggers' }))

    expect(screen.getByText('Slack')).toBeInTheDocument()
    expect(screen.getByText('#triage')).toBeInTheDocument()
    expect(screen.getByText('C123')).toBeInTheDocument()
    expect(screen.getByText('T123')).toBeInTheDocument()
    expect(screen.getByText('Repo One - Thread Based - Bug bash')).toBeInTheDocument()
  })
})
