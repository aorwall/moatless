/**
 * Domain verbs, spoken through the moat CLI.
 *
 * Every method here is one thing a person can do with the platform. Where the
 * CLI has a command for it, the command is what runs; where it does not, the
 * raw call carries it, and the gap is a candidate for a new command rather than
 * a permanent bypass.
 *
 * Response types come from `@moatless/api-client`, generated from the same
 * OpenAPI document the backend serves, so a contract change surfaces here as a
 * type error rather than as a failed assertion.
 */

import type {
  ArtifactRecordResponse,
  ArtifactResponse,
  CreateFeedbackRequest,
  CreateRepositoryRequest,
  CreateTaskBindingRequest,
  FeedbackResponse,
  GetMessagesResponse,
  GithubCommentRequest,
  GithubCommentResponse,
  GithubCreatePrRequest,
  GithubCreatePrResponse,
  GithubReplyRequest,
  ListFeedbackResponse,
  Loop,
  LoopRecentTaskItem,
  ReplyToMessageResponse,
  RepositoryResponse,
  SandboxStatusFullResponse,
  SendMessageResponse,
  SendTaskReplyRequest,
  SendTaskReplyResponse,
  SessionResponse,
  SubagentTranscriptResponse,
  TaskBindingResponse,
  TaskResponse,
  UpdateRepositoryRequest,
  WorkspaceResponse,
} from '@moatless/api-client/sandbox/model'
import type { Moat, RawResponse } from './moat'
import { MoatError, seededUserEmail } from './moat'
import type { Profile } from './profile'
import { scopedName, scopedSlug } from './run-scope'
import type {
  ActivationReach,
  ActivationResponse,
  AdapterConnectionResponse,
  AdapterEventSubscriptionSourceResponse,
  CreatePluginRequest,
  EffectivePluginResponse,
  FileTreeResponse,
  ListTasksFilters,
  ListTasksResponse,
  LoopResponse,
  PluginResponse,
  PluginSkillResponse,
  SearchByNameResult,
  TaskLoopFixtureConfig,
  TaskLoopFixtureResponse,
  TeamMemberResponse,
  TeamResponse,
} from './types'

/**
 * Task creation keyed on a repository template.
 *
 * Hand-written because `CreateTaskRequest` declares no `templateId`, so the
 * generated model no longer carries this shape. The specs below still send it.
 */
interface TemplateTaskRequest {
  templateId: string
  name?: string | null
  message?: string | null
}

/** The shape `moat tasks create` prints. */
interface CreatedTask {
  taskId: string
  status: string
  task: TaskResponse
}

const TEMPLATE_REPOSITORY_NAME = 'Hello World UI'

export class MoatApi {
  constructor(private readonly moat: Moat) {}

  get profile(): Profile {
    return this.moat.profile
  }

  /** The underlying driver, for the raw call and for streaming. */
  get cli(): Moat {
    return this.moat
  }

  /**
   * How long to allow an endpoint that moves the sandbox before it answers.
   *
   * Starting, stopping, closing and deleting all wait on the sandbox inline, so
   * against a cluster they outrun the CLI's own 30-second ceiling. Provisioning
   * is the slowest of them, so its budget covers the rest.
   */
  private get sandboxWork(): number {
    return this.profile.timeouts.sandboxReady
  }

  // =========================================================================
  // Health and identity
  // =========================================================================

  async isHealthy(): Promise<boolean> {
    try {
      const response = await this.moat.raw('GET', '/health')
      return response.status === 200
    } catch {
      return false
    }
  }

  /**
   * `GET /api/v1/auth/me` — the caller's identity plus what the environment
   * still needs from them before it will run their work.
   *
   * `moat whoami` prints the identity only, so the setup block comes from the
   * raw call.
   */
  async me(): Promise<SessionResponse> {
    return this.expect<SessionResponse>(await this.moat.raw('GET', '/api/v1/auth/me'))
  }

  /** The authenticated user's server-assigned id. */
  async userId(): Promise<string> {
    const me = await this.moat.json<{ user?: { id?: string } }>(['whoami'])
    if (!me?.user?.id) {
      throw new Error('Not authenticated — no user id available')
    }
    return me.user.id
  }

  // =========================================================================
  // Repositories
  // =========================================================================

  async listRepositories(): Promise<RepositoryResponse[]> {
    return this.moat.json<RepositoryResponse[]>(['repos', 'list'])
  }

  async getRepository(id: string): Promise<RepositoryResponse> {
    return this.moat.json<RepositoryResponse>(['repos', 'get', id])
  }

  async createRepository(config: CreateRepositoryRequest): Promise<RepositoryResponse> {
    const args = ['repos', 'create', '--name', scopedName(config.name)]
    if (config.remoteUrl) args.push('--git-url', config.remoteUrl)
    if (config.kind) args.push('--kind', config.kind)
    if (config.provider) args.push('--provider', config.provider)
    if (config.defaultBranch) args.push('--branch', config.defaultBranch)
    if (config.description) args.push('--description', config.description)
    if (config.scope) args.push('--scope', config.scope)
    return this.moat.json<RepositoryResponse>(args)
  }

  async deleteRepository(id: string): Promise<void> {
    await this.moat.raw('DELETE', `/api/v1/repositories/${id}`)
  }

  /**
   * The repository tests run tasks against.
   *
   * A profile that names one uses it and fails loudly if it has gone — an
   * environment shared with real work must never have a test invent a
   * repository. A profile that names none creates one from its template.
   */
  async ensureTemplateRepository(): Promise<RepositoryResponse> {
    const { fixture } = this.profile
    if (fixture.repositoryId) {
      try {
        return await this.getRepository(fixture.repositoryId)
      } catch (error) {
        // Only a lookup that reached the API and was answered says anything
        // about the fixture. A driver that could not run at all — no binary,
        // no network — must not be reported as a missing repository, or the
        // first thing anyone does is go looking for the wrong thing.
        if (!(error instanceof MoatError) || error.exitCode === null) {
          throw error
        }
        throw new Error(
          `Fixture repository '${fixture.repositoryId}' is missing from ${this.profile.baseUrl}. ` +
            `Set E2E_REPOSITORY_ID to an existing template repository. (${describeError(error)})`,
        )
      }
    }

    const template = await this.findTemplateRepository()
    if (template) {
      return template
    }
    if (!fixture.templateUrl) {
      throw new Error(
        `Profile '${this.profile.name}' names neither a fixture repository nor a template URL.`,
      )
    }
    try {
      return await this.createRepository({
        name: TEMPLATE_REPOSITORY_NAME,
        kind: 'template',
        remoteUrl: fixture.templateUrl,
      } as CreateRepositoryRequest)
    } catch (error) {
      // Workers create it lazily, so another worker may have won the race (409).
      const created = await this.findTemplateRepository()
      if (created) {
        return created
      }
      throw error
    }
  }

  private async findTemplateRepository(): Promise<RepositoryResponse | undefined> {
    const repositories = await this.listRepositories()
    // By name, not by kind: auth specs create and delete template repositories of their own.
    return repositories.find((repo) => repo.name === scopedName(TEMPLATE_REPOSITORY_NAME))
  }

  // =========================================================================
  // Workspaces
  // =========================================================================

  async getWorkspace(id: string): Promise<WorkspaceResponse> {
    return this.expect<WorkspaceResponse>(await this.moat.raw('GET', `/api/v1/workspaces/${id}`))
  }

  async listWorkspaces(): Promise<WorkspaceResponse[]> {
    return this.moat.json<WorkspaceResponse[]>(['workspaces', 'list'])
  }

  /**
   * The template repository's default Workspace.
   *
   * Every Task and every Loop names a workspace, so a test that only cares
   * about the repository still needs one to run in.
   */
  async ensureTemplateWorkspace(): Promise<WorkspaceResponse> {
    const repo = await this.ensureTemplateRepository()
    return this.ensureWorkspaceFor(repo.id)
  }

  /**
   * The repository's default Workspace.
   *
   * Registering a repository with a GitHub remote creates a git-synced
   * Workspace with source key `default`. When none is found, the repository's
   * `.moatless/` config is synced once before giving up, because the
   * Workspace may predate the repository's remote or its config.
   */
  async ensureWorkspaceFor(repositoryId: string): Promise<WorkspaceResponse> {
    const find = async (): Promise<WorkspaceResponse | undefined> => {
      const live = (await this.listWorkspaces()).filter((workspace) => !workspace.deleted)
      return (
        live.find(
          (workspace) =>
            workspace.sourceKey === 'default' && workspace.sourceRepositoryId === repositoryId,
        ) ??
        live.find(
          (workspace) =>
            workspace.repos.length === 1 && workspace.repos[0].repositoryId === repositoryId,
        )
      )
    }
    const existing = await find()
    if (existing) {
      return existing
    }
    await this.moat.json(['repos', 'sync-config', repositoryId])
    const synced = await find()
    if (!synced) {
      throw new Error(
        `Repository ${repositoryId} has no default Workspace, even after \`moat repos sync-config\`. ` +
          'A default Workspace exists only for a repository with a GitHub remote; a template ' +
          'repository on another host needs a `.moatless/workspaces.json` entry with key "default".',
      )
    }
    return synced
  }

  async deleteWorkspace(id: string): Promise<void> {
    this.expect(await this.deleteWorkspaceRaw(id))
  }

  async deleteWorkspaceRaw(id: string): Promise<RawResponse> {
    return this.moat.raw('DELETE', `/api/v1/workspaces/${id}`)
  }

  // =========================================================================
  // Tasks
  // =========================================================================

  /** Create a task against a repository. */
  async createVibeTask(
    config: (TemplateTaskRequest | { workspaceId: string; name?: string; message?: string }) & {
      /** Harness to run under. Omitted uses the test suite's standard agent. */
      agentProfile?: string
      model?: string
      /**
       * `edit` or `plan`. Omitted means the backend's default. The CLI always
       * creates in edit mode, so asking for a mode goes to the API directly.
       */
      agentMode?: string
      /**
       * Whether the task runs with nobody watching. Defaults to the CLI's own
       * default (true). A test that drives an interactive tool
       * (`AskUserQuestion`, `ExitPlanMode`) has to pass `false` — a headless
       * task's agent is not given them.
       */
      headless?: boolean
      /** Create this task as a member of the named task's warm sandbox. */
      sameSandboxTaskId?: string
      /** Checkout relationship for a same-sandbox member. */
      checkoutMode?: 'shared' | 'isolated'
    },
  ): Promise<TaskResponse> {
    if (config.checkoutMode && !config.sameSandboxTaskId) {
      throw new Error('checkoutMode requires sameSandboxTaskId')
    }

    if (config.agentMode) {
      if (config.sameSandboxTaskId || config.checkoutMode) {
        throw new Error('agentMode task creation cannot use same-sandbox CLI flags')
      }
      const created = this.expect<CreatedTask>(
        await this.moat.raw('POST', '/api/v1/tasks', {
          ...('workspaceId' in config
            ? { workspaceId: config.workspaceId }
            : { repositoryId: config.templateId }),
          name: scopedName(config.name ?? 'task'),
          message: config.message,
          agentProfile: config.agentProfile ?? 'claude-code',
          model: config.model,
          agentMode: config.agentMode,
          headless: config.headless ?? true,
        }),
      )
      return created.task
    }

    const args = ['tasks', 'create']
    args.push('--name', scopedName(config.name ?? 'task'))
    if (config.sameSandboxTaskId) {
      args.push('--same-sandbox', config.sameSandboxTaskId)
    } else if ('workspaceId' in config) {
      args.push('--workspace', config.workspaceId)
    } else {
      // `moat tasks create` refuses a repository without a workspace to run in.
      args.push('--workspace', (await this.ensureWorkspaceFor(config.templateId)).id)
    }
    if (config.message) args.push('--message', config.message)
    args.push('--agent-profile', config.agentProfile ?? 'claude-code')
    if (config.model) args.push('--model', config.model)
    if (config.headless !== undefined) args.push('--headless', String(config.headless))
    if (config.checkoutMode) args.push('--checkout-mode', config.checkoutMode)
    const created = await this.moat.json<CreatedTask>(args)
    return created.task
  }

  /** Create a task and report the status, for assertions on rejection. */
  async createVibeTaskRaw(config: {
    workspaceId: string
    agentProfile?: string
    message?: string
    name?: string
  }): Promise<RawResponse> {
    return this.moat.raw('POST', '/api/v1/tasks', config)
  }

  async getTask(taskId: string): Promise<TaskResponse> {
    return this.moat.json<TaskResponse>(['tasks', 'get', taskId])
  }

  /** Fetch a task and report the status — a task the caller cannot see is a 404. */
  async getTaskRaw(taskId: string): Promise<RawResponse> {
    return this.moat.raw('GET', `/api/v1/tasks/${taskId}`)
  }

  async listTasks(filters?: ListTasksFilters): Promise<ListTasksResponse> {
    const args = ['tasks', 'list']
    if (filters?.status) args.push('--status', filters.status)
    if (filters?.limit !== undefined) args.push('--limit', String(filters.limit))
    if (filters?.offset !== undefined) args.push('--offset', String(filters.offset))
    return this.moat.json<ListTasksResponse>(args)
  }

  async updateTaskMetadata(
    taskId: string,
    data: { name?: string; description?: string },
  ): Promise<TaskResponse> {
    const args = ['tasks', 'update', taskId]
    if (data.name !== undefined) args.push('--name', data.name)
    if (data.description !== undefined) args.push('--description', data.description)
    return this.moat.json<TaskResponse>(args)
  }

  /** Set the Agent profile the task's next turn runs, through `moat tasks update`. */
  async switchProfile(taskId: string, agentProfile: string, model?: string): Promise<TaskResponse> {
    const args = ['tasks', 'update', taskId, '--agent-profile', agentProfile]
    if (model !== undefined) args.push('--model', model)
    return this.moat.json<TaskResponse>(args)
  }

  /**
   * Stores the host's Claude and OpenAI keys as Secrets at `scope`. A switch
   * refuses a profile with no stored credential, and the backend's model
   * gateway, the only route an agent reaches a model by, reads its key there.
   */
  async storeHarnessCredentials(scope: 'user' | 'global' = 'user'): Promise<void> {
    const claude = process.env.CLAUDE_CODE_OAUTH_TOKEN
      ? 'CLAUDE_CODE_OAUTH_TOKEN'
      : 'ANTHROPIC_API_KEY'
    for (const key of [claude, 'OPENAI_API_KEY']) {
      const value = process.env[key]
      if (value) {
        this.expect(
          await this.moat.raw('PUT', '/api/v1/secrets', { scope, key, kind: 'env', value }),
        )
      }
    }
  }

  async switchProfileRaw(taskId: string, agentProfile: string): Promise<RawResponse> {
    return this.moat.raw('PATCH', `/api/v1/tasks/${taskId}`, { agentProfile })
  }

  async setTaskTags(taskId: string, tags: string[]): Promise<RawResponse> {
    return this.moat.raw('PUT', `/api/v1/tasks/${taskId}/tags`, { tags })
  }

  async updateVisibility(taskId: string, visibility: 'private' | 'public'): Promise<TaskResponse> {
    const response = await this.moat.raw('PUT', `/api/v1/tasks/${taskId}/visibility`, {
      visibility,
    })
    return response.body as TaskResponse
  }

  /** Close a task and return it, so a caller can assert on the new status. */
  async closeTask(taskId: string): Promise<TaskResponse> {
    // Closing removes the sandbox before it answers.
    await this.moat.text(['tasks', 'close', taskId, '--close-children', 'false'], {
      timeoutMs: this.sandboxWork,
    })
    return this.getTask(taskId)
  }

  /** Fork a task after `atTurn`, sending `message` as the fork's first Turn. */
  async forkTask(
    taskId: string,
    options: { atTurn?: number; message?: string; agentProfile?: string },
  ): Promise<TaskResponse> {
    const args = ['tasks', 'fork', taskId]
    if (options.atTurn !== undefined) args.push('--at-turn', String(options.atTurn))
    if (options.message) args.push('--message', options.message)
    if (options.agentProfile) args.push('--agent-profile', options.agentProfile)
    const created = await this.moat.json<CreatedTask>(args, { timeoutMs: this.sandboxWork })
    return created.task
  }

  /** Stop a task. Tolerates a task that is not running — teardown calls this blindly. */
  async stopTask(taskId: string): Promise<void> {
    try {
      await this.moat.text(['tasks', 'stop', taskId], { timeoutMs: this.sandboxWork })
    } catch {
      // Already stopped, or already gone.
    }
  }

  /**
   * Delete a task.
   *
   * A task that is already gone is not an error — that is what makes this safe
   * to call from an `afterAll` that may run twice. Anything else is: the sweep
   * reports what it removed, and a refusal it swallowed would read as a success.
   */
  async deleteTask(taskId: string): Promise<void> {
    const response = await this.moat.raw('DELETE', `/api/v1/tasks/${taskId}`, undefined, {
      timeoutMs: this.sandboxWork,
    })
    if (response.status !== 404) {
      this.expect(response)
    }
  }

  async deleteTaskRaw(taskId: string): Promise<RawResponse> {
    return this.moat.raw('DELETE', `/api/v1/tasks/${taskId}`, undefined, {
      timeoutMs: this.sandboxWork,
    })
  }

  // =========================================================================
  // Messages
  // =========================================================================

  async sendMessage(
    taskId: string,
    message: string,
    options?: { resume?: boolean; model?: string },
  ): Promise<void> {
    if (options?.model || options?.resume === false) {
      // The CLI always resumes and never overrides the model; those two
      // cases go through the raw call until a command carries them.
      await this.moat.raw('POST', `/api/v1/tasks/${taskId}/messages`, {
        message,
        resume: options?.resume ?? true,
        model: options?.model,
      })
      return
    }
    await this.moat.text(['send', '--task', taskId, '--text', message])
  }

  async sendMessageRaw(
    taskId: string,
    message: string,
    options?: { resume?: boolean },
  ): Promise<RawResponse> {
    return this.moat.raw('POST', `/api/v1/tasks/${taskId}/messages`, {
      message,
      resume: options?.resume ?? true,
    })
  }

  /**
   * Send a message and get the persisted row's id back.
   *
   * `send` prints human-readable text. A test that has to find the exact
   * message again later — to prove which session acknowledged it — needs the
   * uuid the backend assigned.
   *
   * Sending to a stopped task provisions its sandbox before answering, which
   * on a cluster takes longer than the CLI's default ceiling, so the wait is
   * the profile's sandbox timeout rather than the transport's.
   */
  async postMessage(
    taskId: string,
    body: {
      message: string
      model?: string
      agentMode?: string
      startFresh?: boolean
    },
  ): Promise<SendMessageResponse> {
    return this.expect(
      await this.moat.raw(
        'POST',
        `/api/v1/tasks/${taskId}/messages`,
        { resume: true, ...body },
        { timeoutMs: this.sandboxWork },
      ),
    )
  }

  /**
   * Answer a tool the agent is blocked on, the way the chat UI does.
   *
   * `send` carries text only. Answering names the block being answered, so
   * that the agent resumes the paused turn rather than starting a new one.
   */
  async sendToolResponse(
    taskId: string,
    answer: { toolName: string; toolUseId: string; toolResponse: unknown },
  ): Promise<void> {
    this.expect(
      await this.moat.raw('POST', `/api/v1/tasks/${taskId}/messages`, {
        message: '',
        resume: true,
        ...answer,
      }),
    )
  }

  /**
   * The transcript of the subagent an `Agent` tool call spawned.
   *
   * Subagent messages are folded out of the main transcript and assembled on
   * demand, which is why they are their own call rather than a filter over
   * `getTaskMessages`.
   */
  async getSubagentTranscript(
    taskId: string,
    toolUseId: string,
  ): Promise<SubagentTranscriptResponse> {
    return this.expect(
      await this.moat.raw('GET', `/api/v1/tasks/${taskId}/tool-calls/${toolUseId}/messages`),
    )
  }

  async getTaskMessages(taskId: string): Promise<GetMessagesResponse> {
    const response = await this.moat.raw('GET', `/api/v1/tasks/${taskId}/messages`)
    return response.body as GetMessagesResponse
  }

  /**
   * The conversation as the chat list asks for it.
   *
   * The list always sends `?summary=true`, and the projection behind that flag
   * drops content the full response carries — so a card that renders from a
   * tool call's input can go blank without the full response ever changing.
   */
  async getTaskMessagesSummary(taskId: string): Promise<GetMessagesResponse> {
    const response = await this.moat.raw('GET', `/api/v1/tasks/${taskId}/messages?summary=true`)
    return response.body as GetMessagesResponse
  }

  /** Reply to one message through the route its source decides. */
  async replyToMessage(messageId: string, text: string): Promise<ReplyToMessageResponse> {
    return this.moat.json<ReplyToMessageResponse>([
      'messages',
      'reply',
      '--message',
      messageId,
      '--text',
      text,
    ])
  }

  // =========================================================================
  // Artifacts
  // =========================================================================

  /** Publish `path` as the next version of the Artifact `slug` on `taskId`. */
  async publishArtifact(
    taskId: string,
    path: string,
    options: { title: string; slug: string; data?: boolean; notify?: boolean },
  ): Promise<ArtifactResponse> {
    const args = ['artifact', 'publish', path, '--task', taskId]
    args.push('--title', options.title, '--slug', options.slug)
    if (options.data) args.push('--data')
    if (options.notify) args.push('--notify')
    return this.moat.json<ArtifactResponse>(args)
  }

  /** The record saved under `key`, read as the Task's agent reads it. */
  async getArtifactRecord(
    taskId: string,
    slug: string,
    key: string,
  ): Promise<ArtifactRecordResponse> {
    return this.moat.json<ArtifactRecordResponse>([
      'artifact',
      'data',
      'get',
      slug,
      key,
      '--task',
      taskId,
    ])
  }

  /**
   * Write a record as the served page does: with its page token and no
   * credential, so the write is the reader's and not this driver's.
   */
  async writePageRecord(
    token: string,
    request: { op: 'save'; key: string; data: unknown },
  ): Promise<ArtifactRecordResponse> {
    return this.expect(
      await this.moat.unauthenticated().raw('POST', `/api/artifact-data/${token}/records`, request),
    )
  }

  // =========================================================================
  // The GitHub proxy
  //
  // The backend posts to GitHub on the task's behalf so that the reply carries
  // the task's identity and binds the task to what it replied to. `moat gh`
  // drives these from a checkout; a test names the repository outright.
  // =========================================================================

  async githubCreatePrRaw(
    owner: string,
    repo: string,
    req: GithubCreatePrRequest,
  ): Promise<RawResponse> {
    return this.moat.raw('POST', `/api/v1/github/repos/${owner}/${repo}/pulls`, req)
  }

  async githubCreatePr(
    owner: string,
    repo: string,
    req: GithubCreatePrRequest,
  ): Promise<GithubCreatePrResponse> {
    return this.expect(await this.githubCreatePrRaw(owner, repo, req))
  }

  async githubIssueCommentRaw(
    owner: string,
    repo: string,
    issueNumber: number,
    req: GithubCommentRequest,
  ): Promise<RawResponse> {
    return this.moat.raw(
      'POST',
      `/api/v1/github/repos/${owner}/${repo}/issues/${issueNumber}/comments`,
      req,
    )
  }

  async githubIssueComment(
    owner: string,
    repo: string,
    issueNumber: number,
    req: GithubCommentRequest,
  ): Promise<GithubCommentResponse> {
    return this.expect(await this.githubIssueCommentRaw(owner, repo, issueNumber, req))
  }

  async githubReplyRaw(
    owner: string,
    repo: string,
    prNumber: number,
    commentId: number,
    req: GithubReplyRequest,
  ): Promise<RawResponse> {
    return this.moat.raw(
      'POST',
      `/api/v1/github/repos/${owner}/${repo}/pulls/${prNumber}/comments/${commentId}/replies`,
      req,
    )
  }

  async githubReply(
    owner: string,
    repo: string,
    prNumber: number,
    commentId: number,
    req: GithubReplyRequest,
  ): Promise<GithubCommentResponse> {
    return this.expect(await this.githubReplyRaw(owner, repo, prNumber, commentId, req))
  }

  // =========================================================================
  // Task bindings and replies
  //
  // A binding is what makes a task answerable: it says which external subject —
  // a webhook thread, a PR, a Slack thread — a reply should go back to. A task
  // with none has nowhere to send, and a task with several has to be told which.
  // =========================================================================

  async listTaskBindings(taskId: string): Promise<TaskBindingResponse[]> {
    return this.moat.json<TaskBindingResponse[]>(['bindings', 'list', '--task', taskId])
  }

  async createTaskBinding(
    taskId: string,
    config: CreateTaskBindingRequest,
  ): Promise<TaskBindingResponse> {
    const args = [
      'bindings',
      'create',
      '--task',
      taskId,
      '--adapter',
      config.adapterKind,
      '--connection-id',
      config.connectionId,
      '--subject-kind',
      config.subjectKind,
      '--subject-key',
      config.subjectKey,
    ]
    if (config.loopId) args.push('--loop-id', config.loopId)
    if (config.destinationConfig)
      args.push('--destination-config', JSON.stringify(config.destinationConfig))
    if (config.metadata) args.push('--metadata', JSON.stringify(config.metadata))
    return this.moat.json<TaskBindingResponse>(args)
  }

  async sendTaskReply(
    taskId: string,
    request: SendTaskReplyRequest,
  ): Promise<SendTaskReplyResponse> {
    return this.expect<SendTaskReplyResponse>(await this.sendTaskReplyRaw(taskId, request))
  }

  async sendTaskReplyRaw(taskId: string, request: SendTaskReplyRequest): Promise<RawResponse> {
    return this.moat.raw('POST', `/api/v1/tasks/${taskId}/reply`, request)
  }

  // =========================================================================
  // Feedback
  // =========================================================================

  /**
   * Submit feedback about a task, a message, or a single tool call.
   *
   * `moat feedback` speaks only for the task it is running inside and only at
   * task level, so a suite that has to name a surface and a task goes through
   * the raw call.
   */
  async submitFeedback(request: CreateFeedbackRequest): Promise<FeedbackResponse> {
    return this.expect<FeedbackResponse>(await this.submitFeedbackRaw(request))
  }

  async submitFeedbackRaw(request: CreateFeedbackRequest): Promise<RawResponse> {
    return this.moat.raw('POST', '/api/v1/feedback', request)
  }

  async listTaskFeedback(taskId: string): Promise<ListFeedbackResponse> {
    return this.expect<ListFeedbackResponse>(
      await this.moat.raw('GET', `/api/v1/tasks/${taskId}/feedback`),
    )
  }

  // =========================================================================
  // Loops
  // =========================================================================

  /**
   * A schedule loop, created paused unless the caller says otherwise.
   *
   * A schedule is the one loop that acts on its own: left active on a shared
   * cluster, a leaked one keeps spawning tasks long after the run that made
   * it. Tests that only exercise the record want it inert.
   */
  async createScheduleLoop(config: {
    name: string
    workspaceId: string
    cronExpression: string
    messageTemplate: string
    timezone?: string
    routingMode?: string
    taskName?: string
    prompt?: string
    agentProfile?: string
    active?: boolean
  }): Promise<Loop> {
    const args = [
      'loops',
      'create-schedule',
      '--name',
      scopedName(config.name),
      '--workspace-id',
      config.workspaceId,
      '--cron-expression',
      config.cronExpression,
      '--message-template',
      config.messageTemplate,
      '--active',
      String(config.active ?? false),
    ]
    if (config.timezone) args.push('--timezone', config.timezone)
    if (config.routingMode) args.push('--routing-mode', config.routingMode)
    // Scoped so that a task the schedule does spawn carries the run prefix
    // and the sweep can find it.
    if (config.taskName) args.push('--task-name', scopedName(config.taskName))
    if (config.prompt) args.push('--prompt', config.prompt)
    if (config.agentProfile) args.push('--agent-profile', config.agentProfile)
    return this.moat.json<Loop>(args)
  }

  async updateLoop(
    id: string,
    data: {
      active?: boolean
      prompt?: string
      taskName?: string
      cronExpression?: string
      timezone?: string
      messageTemplate?: string
    },
  ): Promise<Loop> {
    const args = ['loops', 'update', '--id', id]
    if (data.active !== undefined) args.push('--active', String(data.active))
    if (data.prompt !== undefined) args.push('--prompt', data.prompt)
    if (data.taskName !== undefined) args.push('--task-name', data.taskName)
    if (data.cronExpression !== undefined) args.push('--cron-expression', data.cronExpression)
    if (data.timezone !== undefined) args.push('--timezone', data.timezone)
    if (data.messageTemplate !== undefined) args.push('--message-template', data.messageTemplate)
    return this.moat.json<Loop>(args)
  }

  async listLoops(filters?: { kind?: string; adapter?: string }): Promise<Loop[]> {
    const args = ['loops', 'list']
    if (filters?.kind) args.push('--kind', filters.kind)
    if (filters?.adapter) args.push('--adapter', filters.adapter)
    return this.moat.json<Loop[]>(args)
  }

  async deleteLoop(id: string): Promise<void> {
    // `--yes` because there is nobody at the terminal to confirm.
    await this.moat.text(['loops', 'delete', '--id', id, '--yes'])
  }

  /**
   * The tasks a loop recently produced.
   *
   * Driven by the task's own `loopId`, not by a binding — a schedule loop
   * never creates one, and its tasks still have to surface here.
   */
  async listLoopRecentTasks(id: string, limit = 10): Promise<LoopRecentTaskItem[]> {
    return this.expect<LoopRecentTaskItem[]>(
      await this.moat.raw('GET', `/api/v1/loops/${id}/tasks?limit=${limit}`),
    )
  }

  // =========================================================================
  // Sandbox
  // =========================================================================

  async getWorkspaceStatus(taskId: string): Promise<SandboxStatusFullResponse> {
    return this.moat.json<SandboxStatusFullResponse>(['tasks', 'status', taskId])
  }

  async provisionSandbox(taskId: string): Promise<unknown> {
    const response = await this.moat.raw(
      'POST',
      `/api/sandbox/v1/tasks/${taskId}/provision`,
      undefined,
      { timeoutMs: this.sandboxWork },
    )
    return response.body
  }

  async stopSandbox(taskId: string): Promise<void> {
    await this.moat.raw('POST', `/api/sandbox/v1/tasks/${taskId}/stop`, undefined, {
      timeoutMs: this.sandboxWork,
    })
  }

  /** Start a stopped sandbox — the task start path provisions it. */
  async startSandbox(taskId: string): Promise<void> {
    await this.moat.text(['tasks', 'start', taskId], { timeoutMs: this.sandboxWork })
  }

  async restartSandbox(taskId: string): Promise<void> {
    await this.moat.text(['tasks', 'restart', taskId], {
      timeoutMs: this.sandboxWork,
    })
  }

  async cleanupSandbox(taskId: string): Promise<void> {
    await this.moat.raw('POST', `/api/sandbox/v1/tasks/${taskId}/cleanup`, undefined, {
      timeoutMs: this.sandboxWork,
    })
  }

  // =========================================================================
  // Files
  // =========================================================================

  async readWorkspaceFile(taskId: string, filePath: string): Promise<string | null> {
    const response = await this.moat.raw(
      'GET',
      `/api/sandbox/v1/tasks/${taskId}/files/read?path=${encodeURIComponent(filePath)}`,
    )
    if (response.status >= 400) {
      return null
    }
    return (response.body as { content?: string })?.content ?? null
  }

  async listWorkspaceFiles(taskId: string, dirPath = '.', recursive = false): Promise<string[]> {
    const query = `path=${encodeURIComponent(dirPath)}&recursive=${recursive}`
    const response = await this.moat.raw(
      'GET',
      `/api/sandbox/v1/tasks/${taskId}/files/list?${query}`,
    )
    if (response.status >= 400) {
      return []
    }
    return (response.body as { files?: string[] })?.files ?? []
  }

  async getFileTree(taskId: string, path?: string, recursive = false): Promise<FileTreeResponse> {
    const parts = [
      path ? `path=${encodeURIComponent(path)}` : '',
      recursive ? 'recursive=true' : '',
    ].filter(Boolean)
    const query = parts.length > 0 ? `?${parts.join('&')}` : ''
    return this.expect<FileTreeResponse>(
      await this.moat.raw('GET', `/api/sandbox/v1/tasks/${taskId}/files/tree${query}`),
    )
  }

  async writeFile(taskId: string, path: string, content: string): Promise<void> {
    this.expect(
      await this.moat.raw('POST', `/api/sandbox/v1/tasks/${taskId}/files/write`, {
        path,
        content,
      }),
    )
  }

  async searchFilesByName(taskId: string, query: string): Promise<SearchByNameResult> {
    return this.expect<SearchByNameResult>(
      await this.moat.raw('POST', `/api/sandbox/v1/tasks/${taskId}/files/search-by-name`, {
        query,
      }),
    )
  }

  // =========================================================================
  // Repositories, continued
  // =========================================================================

  async getRepositoryRaw(id: string): Promise<RawResponse> {
    return this.moat.raw('GET', `/api/v1/repositories/${id}`)
  }

  async createRepositoryRaw(config: Partial<CreateRepositoryRequest>): Promise<RawResponse> {
    return this.moat.raw('POST', '/api/v1/repositories', config)
  }

  async updateRepository(id: string, data: UpdateRepositoryRequest): Promise<RepositoryResponse> {
    return this.expect<RepositoryResponse>(
      await this.moat.raw('PUT', `/api/v1/repositories/${id}`, data),
    )
  }

  async updateRepositoryRaw(id: string, data: UpdateRepositoryRequest): Promise<RawResponse> {
    return this.moat.raw('PUT', `/api/v1/repositories/${id}`, data)
  }

  async deleteRepositoryRaw(id: string): Promise<RawResponse> {
    return this.moat.raw('DELETE', `/api/v1/repositories/${id}`)
  }

  // =========================================================================
  // Loops — created as a connection plus an adapter-event subscription
  // =========================================================================

  async createTaskTriggerFixture(config: TaskLoopFixtureConfig): Promise<TaskLoopFixtureResponse> {
    return toTaskTriggerFixture(
      this.expect<LoopResponse>(await this.createTaskTriggerFixtureRaw(config)),
    )
  }

  async listAdapterConnections(): Promise<AdapterConnectionResponse[]> {
    return this.expect<AdapterConnectionResponse[]>(
      await this.moat.raw('GET', '/api/v1/adapters/connections'),
    )
  }

  /**
   * Remove a connection, reporting how the API answered.
   *
   * Raw rather than checked: the endpoint is newer than the deployed backend
   * the suite runs against, so the sweep has to be able to tell "the
   * environment does not have this yet" from "the delete failed" and say so
   * rather than throwing.
   */
  async deleteAdapterConnectionRaw(connectionId: string): Promise<RawResponse> {
    return this.moat.raw('DELETE', `/api/v1/adapters/connections/${connectionId}`)
  }

  /** Create one and report how the API answered, for a rejection assertion. */
  async createTaskTriggerFixtureRaw(config: TaskLoopFixtureConfig): Promise<RawResponse> {
    const adapterKind = config.adapterKind ?? config.provider ?? 'webhook'
    // A live loop routes inbound events into new tasks, so on a shared
    // environment it is the one leftover that keeps doing something. Both the
    // matcher and the name carry the run, so the sweep can find it.
    const sourceId = scopedSlug(config.sourceId)
    const sourceName = scopedName(config.sourceName ?? config.sourceId)
    // An active Loop impersonates a real user when it spawns tasks; the backend
    // rejects activation without one. Default to the authenticated caller.
    const runAsUserId = config.runAsUserId ?? (await this.userId())
    const connection = this.expect<AdapterConnectionResponse>(
      await this.moat.raw('POST', '/api/v1/adapters/connections', {
        adapterKind,
        externalAccountId: sourceId,
        sourceName,
        connectionKind:
          config.connectionKind ?? (adapterKind === 'webhook' ? 'webhook' : 'bot_api'),
        webhookSecret: adapterConfigSecret(config.adapterConfig),
      }),
    )

    return this.moat.raw('POST', '/api/v1/loops', {
      name: sourceName,
      kind: 'adapter_event',
      active: true,
      config: {
        workspaceId: config.workspaceId,
        routingMode: normalizeRoutingMode(config.routingMode),
        prompt: config.promptTemplate ?? null,
        taskName: config.taskName ?? null,
        runAsUserId,
      },
      source: {
        adapterKind,
        connectionId: connection.id,
        sourceMatcher: config.sourceMatcher ?? sourceId,
        sourceName,
        eventFilter: {},
        adapterConfig: config.adapterConfig ?? {},
        allowDirectSend: false,
        closeTaskOnClose: true,
        notifyOnSuccess: false,
      },
    })
  }

  async getTaskTriggerFixture(id: string): Promise<TaskLoopFixtureResponse> {
    const loop = this.expect<LoopResponse>(await this.moat.raw('GET', `/api/v1/loops/${id}`))
    return toTaskTriggerFixture(loop)
  }

  async updateTaskTriggerFixture(
    id: string,
    data: {
      sourceName?: string
      routingMode?: string
      promptTemplate?: string | null
      taskName?: string | null
      active?: boolean
      adapterConfig?: Record<string, unknown>
    },
  ): Promise<TaskLoopFixtureResponse> {
    const current = await this.getTaskTriggerFixture(id)
    // Activation is not part of the loop update contract — it moves through the
    // dedicated activate/pause endpoints (activation must name a run-as user).
    if (data.active !== undefined && data.active !== current.active) {
      if (data.active) {
        this.expect(
          await this.moat.raw('POST', `/api/v1/loops/${id}/activate`, {
            runAsUserId: await this.userId(),
          }),
        )
      } else {
        this.expect(await this.moat.raw('POST', `/api/v1/loops/${id}/pause`))
      }
    }
    const loop = this.expect<LoopResponse>(
      await this.moat.raw('PUT', `/api/v1/loops/${id}`, {
        config: {
          routingMode: data.routingMode
            ? normalizeRoutingMode(data.routingMode)
            : current.routingMode,
          prompt:
            data.promptTemplate === undefined
              ? (current.promptTemplate ?? null)
              : data.promptTemplate,
          taskName: data.taskName === undefined ? (current.taskName ?? null) : data.taskName,
        },
        source: {
          sourceName:
            data.sourceName === undefined
              ? (current.sourceName ?? null)
              : scopedName(data.sourceName),
          adapterConfig: data.adapterConfig ?? current.adapterConfig ?? {},
        },
      }),
    )
    return toTaskTriggerFixture(loop)
  }

  async deleteTaskTriggerFixture(id: string): Promise<void> {
    this.expect(await this.moat.raw('DELETE', `/api/v1/loops/${id}`))
  }

  async deleteTaskTriggerFixtureRaw(id: string): Promise<RawResponse> {
    return this.moat.raw('DELETE', `/api/v1/loops/${id}`)
  }

  async listTaskTriggerFixtures(filters?: {
    adapterKind?: string
    provider?: string
    workspaceId?: string
  }): Promise<TaskLoopFixtureResponse[]> {
    const params = new URLSearchParams({ kind: 'adapter_event' })
    const adapterKind = filters?.adapterKind ?? filters?.provider
    if (adapterKind) params.set('adapterKind', adapterKind)

    const loops = this.expect<LoopResponse[]>(
      await this.moat.raw('GET', `/api/v1/loops?${params.toString()}`),
    )
    return loops
      .map(toTaskTriggerFixture)
      .filter((fixture) => !filters?.workspaceId || fixture.workspaceId === filters.workspaceId)
  }

  /** Deliver a message through a webhook connection. */
  async sendWebhookMessage(
    triggerId: string,
    payload: {
      text: string
      threadId?: string
      userId?: string
      metadata?: Record<string, unknown>
      deliveryId?: string
    },
  ): Promise<RawResponse> {
    // The text is stamped like a name. This delivery creates a task the backend
    // titles from the message itself, so the name it ends up with carries no
    // prefix — the description, which holds this text verbatim, is the only place
    // a sweep running in a different sandbox can still read the run off it.
    return this.moat.raw('POST', `/api/v1/adapters/webhook/${triggerId}`, {
      ...payload,
      text: scopedName(payload.text),
    })
  }

  /** Deliver an arbitrary webhook payload, for malformed-input assertions. */
  async sendWebhookMessageRaw(
    triggerId: string,
    payload: unknown,
    headers?: Record<string, string>,
  ): Promise<RawResponse> {
    return this.moat.raw(
      'POST',
      `/api/v1/adapters/webhook/${triggerId}`,
      payload,
      headers ? { headers } : undefined,
    )
  }

  // =========================================================================
  // Teams
  // =========================================================================

  async createTeam(req: {
    name: string
    slug: string
    description?: string
  }): Promise<TeamResponse> {
    return this.expect<TeamResponse>(
      await this.moat.raw('POST', '/api/v1/teams', {
        ...req,
        name: scopedName(req.name),
        slug: scopedSlug(req.slug),
      }),
    )
  }

  async createTeamRaw(req: {
    name?: string
    slug?: string
    description?: string
  }): Promise<RawResponse> {
    return this.moat.raw('POST', '/api/v1/teams', req)
  }

  async getTeam(id: string): Promise<TeamResponse> {
    return this.expect<TeamResponse>(await this.moat.raw('GET', `/api/v1/teams/${id}`))
  }

  async updateTeam(
    id: string,
    data: { name?: string; description?: string },
  ): Promise<TeamResponse> {
    return this.expect<TeamResponse>(await this.moat.raw('PUT', `/api/v1/teams/${id}`, data))
  }

  async deleteTeam(id: string): Promise<void> {
    this.expect(await this.moat.raw('DELETE', `/api/v1/teams/${id}`))
  }

  async listTeams(): Promise<TeamResponse[]> {
    return this.moat.json<TeamResponse[]>(['teams', 'list'])
  }

  async addTeamMember(
    teamId: string,
    userId: string,
    role = 'member',
  ): Promise<TeamMemberResponse> {
    return this.expect<TeamMemberResponse>(
      await this.moat.raw('POST', `/api/v1/teams/${teamId}/members`, { userId, role }),
    )
  }

  async removeTeamMember(teamId: string, userId: string): Promise<void> {
    this.expect(await this.moat.raw('DELETE', `/api/v1/teams/${teamId}/members/${userId}`))
  }

  async listTeamMembers(teamId: string): Promise<TeamMemberResponse[]> {
    return this.expect<TeamMemberResponse[]>(
      await this.moat.raw('GET', `/api/v1/teams/${teamId}/members`),
    )
  }

  // =========================================================================
  // Plugins
  // =========================================================================

  async createPlugin(req: CreatePluginRequest): Promise<PluginResponse> {
    return this.moat.json<PluginResponse>([
      'plugins',
      'create',
      '--git-url',
      req.gitUrl,
      '--name',
      scopedName(req.name),
    ])
  }

  async listPlugins(): Promise<PluginResponse[]> {
    return this.moat.json<PluginResponse[]>(['plugins', 'list'])
  }

  async deletePlugin(pluginId: string): Promise<void> {
    this.expect(await this.moat.raw('DELETE', `/api/v1/plugins/${pluginId}`))
  }

  async listPluginSkills(pluginId: string): Promise<PluginSkillResponse[]> {
    return this.expect<PluginSkillResponse[]>(
      await this.moat.raw('GET', `/api/v1/plugins/${pluginId}/skills`),
    )
  }

  async listPluginActivations(
    pluginId: string,
    workspaceId?: string,
  ): Promise<ActivationResponse[]> {
    const qs = workspaceId === undefined ? '' : `?workspaceId=${workspaceId}`
    return this.expect<ActivationResponse[]>(
      await this.moat.raw('GET', `/api/v1/plugins/${pluginId}/activations${qs}`),
    )
  }

  /** What resolves for the caller, in a workspace when one is named. */
  async listEffectivePlugins(workspaceId?: string): Promise<EffectivePluginResponse[]> {
    const qs = workspaceId === undefined ? '' : `?workspaceId=${workspaceId}`
    return this.expect<EffectivePluginResponse[]>(
      await this.moat.raw('GET', `/api/v1/plugins/effective${qs}`),
    )
  }

  async setPluginActivation(
    pluginId: string,
    body: {
      reach: ActivationReach
      skillName?: string | null
      workspaceId?: string
      enabled: boolean
    },
  ): Promise<void> {
    this.expect(await this.moat.raw('PUT', `/api/v1/plugins/${pluginId}/activation`, body))
  }

  async clearPluginActivation(
    pluginId: string,
    reach: ActivationReach,
    skillName?: string,
    workspaceId?: string,
  ): Promise<void> {
    const params = new URLSearchParams({ reach })
    if (skillName !== undefined) {
      params.set('skillName', skillName)
    }
    if (workspaceId !== undefined) {
      params.set('workspaceId', workspaceId)
    }
    this.expect(
      await this.moat.raw('DELETE', `/api/v1/plugins/${pluginId}/activation?${params.toString()}`),
    )
  }

  // =========================================================================
  // Auth
  // =========================================================================

  /**
   * Create a local password user. Idempotent — an existing login answers 409,
   * which is what a fixture wants.
   *
   * Only possible where the profile reports `seedUsers`: a deployed auth
   * provider owns its own identities and refuses to mint one here.
   */
  async ensureUser(
    login: string,
    password: string,
    role: 'user' | 'admin' = 'user',
  ): Promise<void> {
    const response = await this.moat.raw('POST', '/api/v1/users', {
      isBot: false,
      login,
      email: seededUserEmail(login),
      password,
      role,
      mustChangePassword: false,
    })
    if (response.status >= 400 && response.status !== 409) {
      throw new Error(
        `Failed to create user '${login}': ${response.status} ${JSON.stringify(response.body)}`,
      )
    }
  }

  /** The ForwardAuth verify endpoint, whose answer is a status. */
  async verifyAuth(extraHeaders?: Record<string, string>): Promise<RawResponse> {
    return this.moat.raw(
      'GET',
      '/api/v1/auth/verify',
      undefined,
      extraHeaders ? { headers: extraHeaders } : undefined,
    )
  }

  /**
   * Unwrap a response the caller expects to have succeeded.
   *
   * The raw call never throws on a rejection — that is its purpose — so a verb
   * built on it has to decide for itself, and a verb that is not asking about
   * rejection wants the failure loud and immediate.
   */
  private expect<T>(response: RawResponse): T {
    if (response.status >= 400) {
      throw new Error(
        `Request failed with status ${response.status}: ${JSON.stringify(response.body)}`,
      )
    }
    return response.body as T
  }
}

function adapterConfigSecret(adapterConfig?: Record<string, unknown>): string | null {
  const secret = adapterConfig?.secret_token ?? adapterConfig?.webhook_secret
  return typeof secret === 'string' && secret.length > 0 ? secret : null
}

function normalizeRoutingMode(mode: string): string {
  return mode === 'per_message' || mode === 'thread_based' ? 'by_subject' : mode
}

function isAdapterEventSource(
  source: LoopResponse['source'],
): source is AdapterEventSubscriptionSourceResponse {
  return 'adapterKind' in source
}

function toTaskTriggerFixture(loop: LoopResponse): TaskLoopFixtureResponse {
  if (!isAdapterEventSource(loop.source)) {
    throw new Error(`Loop is not an adapter-event subscription loop: ${loop.id}`)
  }
  return {
    id: loop.id,
    connectionId: loop.source.connectionId,
    adapterKind: loop.source.adapterKind,
    sourceId: loop.source.sourceMatcher,
    sourceName: loop.source.sourceName ?? undefined,
    workspaceId: loop.config.workspaceId,
    routingMode: loop.config.routingMode,
    promptTemplate: loop.config.prompt ?? undefined,
    taskName: loop.config.taskName ?? undefined,
    active: loop.active,
    adapterConfig: loop.source.adapterConfig ?? undefined,
    ongoingTaskId: loop.config.ongoingTaskId ?? undefined,
    createdBy: loop.createdBy,
    createdAt: loop.createdAt,
    updatedAt: loop.updatedAt,
  }
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
