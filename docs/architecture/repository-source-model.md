# Repository Source Model

## Problem
Repositories currently conflate user-managed directories and remote Git references. The shared `RepositoryConfigSchema` allows `path` and `remoteUrl` to coexist without clear invariants, making it difficult to enforce proper constraints. This leads to git commands being run in non-existent directories, unclear ownership semantics, and blocks the ability to support multiple Git providers (GitHub, GitLab, self-hosted).

Additionally, template repositories (created from predefined sources) are not properly distinguished from regular repositories, causing confusion about their lifecycle and management.

## Goals
1. Represent repositories using a discriminated union (local, remote, template) with clear, enforceable invariants
2. Ensure remote repositories have NO persistent clone—use provider APIs for metadata operations instead
3. Distinguish template repositories as a separate type with explicit lifecycle management
4. Support multiple Git providers (GitHub, GitLab, generic) from the start
5. Provide a transparent migration path for existing data and API consumers

## Proposed Solution

### Domain Model

Introduce a **three-way discriminated union** based on repository kind:

```typescript
type RepositoryConfig = LocalRepository | RemoteRepository | TemplateRepository

interface LocalRepository {
  kind: 'local'
  path: string                    // REQUIRED: User-specified filesystem path
  useGitWorktrees: boolean        // Optional: Use worktrees for task isolation
  originalBranch?: string         // Optional: Default branch name
  // NO remoteUrl allowed (strictly local)
  // ... shared fields (name, description, etc.)
}

interface RemoteRepository {
  kind: 'remote'
  remote: {
    url: string                   // REQUIRED: Git remote URL
    provider: 'github' | 'gitlab'  // REQUIRED: Provider type
    defaultBranch?: string        // Optional: Default branch name
  }
  // NO path field (no persistent clone)
  // NO useGitWorktrees (always clone per-task)
  // ... shared fields (name, description, etc.)
}

interface TemplateRepository {
  kind: 'template'
  templateId: string              // REQUIRED: Template identifier
  path: string                    // REQUIRED: Where template was cloned
  useGitWorktrees: boolean        // Optional: Use worktrees for task isolation
  originalBranch?: string         // Optional: Default branch name
  // ... shared fields (name, description, etc.)
}
```

**Key Invariants:**
- Local repos: MUST have `path`, MUST NOT have `remote`
- Remote repos: MUST have `remote.url` and `remote.provider`, MUST NOT have `path`
- Template repos: MUST have `templateId` and `path`, represent cloned templates
- No hybrid types: A repository is strictly one kind

**Shared Fields** (all repository types):
- `id`: Unique identifier
- `name`: Display name
- `description`: Optional description
- `syncWithRepoConfig`: Sync from `.moatless/config.json`
- `systemPrompt`: Additional system prompt
- `codeExecution`: Code execution config
- `previewConfig`: Preview configuration
- `createdAt`, `updatedAt`: Timestamps
- `deleted`: Soft-delete flag

### Persistence

**Database Schema** (`apps/backend/src/features/repositories/db/schema.ts`):
```typescript
export const repositories = sqliteTable('repositories', {
  id: text('id').primaryKey(),
  name: text('name').notNull(),
  kind: text('kind').notNull(), // 'local' | 'remote' | 'template'

  // Local/Template fields
  path: text('path'),           // Required for local/template, NULL for remote

  // Remote fields
  remoteUrl: text('remote_url'),        // Required for remote, NULL otherwise
  provider: text('provider'),           // 'github' | 'gitlab'

  // Template fields
  templateId: text('template_id'),      // Required for template, NULL otherwise

  // Workspace modes (local/template only)
  useGitWorktrees: integer('use_git_worktrees', { mode: 'boolean' }),

  // Shared fields
  description: text('description'),
  originalBranch: text('original_branch'),
  syncWithRepoConfig: integer('sync_with_repo_config', { mode: 'boolean' }),
  systemPrompt: text('system_prompt'),

  // Code execution fields (flattened from CodeExecution)
  setupCommands: text('setup_commands', { mode: 'json' }).$type<string[]>(),
  dockerImage: text('docker_image'),
  envVars: text('env_vars', { mode: 'json' }).$type<Record<string, string>>(),

  // Preview configuration fields (flattened from PreviewConfig)
  serverStartCommands: text('server_start_commands', { mode: 'json' }).$type<string[]>(),
  buildCommand: text('build_command'),
  buildOutputDirectory: text('build_output_directory').default('dist'),
  takeScreenshots: integer('take_screenshots', { mode: 'boolean' }),
  enableInspector: integer('enable_inspector', { mode: 'boolean' }).default(true),
  startupTimeoutSeconds: integer('startup_timeout_seconds').default(30),

  deleted: integer('deleted', { mode: 'boolean' }),
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
  updatedAt: integer('updated_at', { mode: 'timestamp' }).notNull(),
})
```

**Validation:**
- Enforce discriminated union constraints in Zod schemas
- Local: `path` required, `remoteUrl`/`provider`/`templateId` must be NULL
- Remote: `remoteUrl`/`provider` required, `path`/`templateId`/`useGitWorktrees` must be NULL
- Template: `templateId`/`path` required, `remoteUrl`/`provider` must be NULL

### Schema Updates

**File:** `packages/types/src/schemas/repository.schemas.ts`

```typescript
// Shared base schema
const BaseRepositorySchema = z.object({
  id: z.string(),
  name: repositoryNameSchema,
  description: z.string().optional(),
  syncWithRepoConfig: z.boolean().optional(),
  systemPrompt: z.string().optional(),
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  deleted: z.boolean().optional(),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

// Local repository schema
const LocalRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('local'),
  path: z.string(),                              // REQUIRED
  useGitWorktrees: z.boolean().default(false),
  originalBranch: z.string().optional(),
})

// Remote repository schema
const RemoteRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('remote'),
  remote: z.object({
    url: z.string(),                             // REQUIRED
    provider: z.enum(['github', 'gitlab']),  // REQUIRED
    defaultBranch: z.string().optional(),
  }),
  // Explicitly forbid local-only fields
})

// Template repository schema
const TemplateRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('template'),
  templateId: z.string(),                        // REQUIRED
  path: z.string(),                              // REQUIRED
  useGitWorktrees: z.boolean().default(false),
  originalBranch: z.string().optional(),
})

// Discriminated union
export const RepositoryConfigSchema = z.discriminatedUnion('kind', [
  LocalRepositorySchema,
  RemoteRepositorySchema,
  TemplateRepositorySchema,
])

// Creation schemas (separate for each type)
export const CreateLocalRepositorySchema = z.object({
  name: repositoryNameSchema,
  kind: z.literal('local'),
  path: z.string(),
  description: z.string().optional(),
  useGitWorktrees: z.boolean().default(false),
  originalBranch: z.string().optional(),
  // ... other shared fields
})

export const CreateRemoteRepositorySchema = z.object({
  name: repositoryNameSchema,
  kind: z.literal('remote'),
  remoteUrl: z.string(),
  provider: z.enum(['github', 'gitlab']),
  description: z.string().optional(),
  // ... other shared fields
})

export const CreateTemplateRepositorySchema = z.object({
  name: repositoryNameSchema,
  kind: z.literal('template'),
  templateId: z.string(),
  description: z.string().optional(),
  // ... other shared fields
  // Note: path is NOT provided by user, generated during creation
})

// Union of creation requests
export const CreateRepositorySchema = z.discriminatedUnion('kind', [
  CreateLocalRepositorySchema,
  CreateRemoteRepositorySchema,
  CreateTemplateRepositorySchema,
])
```

**Provider-Specific URL Validation:**
```typescript
const githubUrlSchema = z.string().regex(
  /^https:\/\/github\.com\/[^/]+\/[^/.]+(?:\.git)?$/,
  'Must be a valid GitHub HTTPS URL'
)

const gitlabUrlSchema = z.string().regex(
  /^https:\/\/gitlab\.com\/[^/]+\/[^/.]+(?:\.git)?$/,
  'Must be a valid GitLab HTTPS URL'
)

// Apply in remote schema based on provider
```

### Service-Layer Flow

**File:** `apps/backend/src/features/repositories/services/repository.service.ts`

#### Repository Creation

**Split into three distinct creation methods:**

```typescript
async createLocalRepository(
  request: CreateLocalRepositoryRequest
): Promise<LocalRepository> {
  // 1. Validate path exists and is a git repository
  await this.gitService.validateRepository(request.path)

  // 2. Check name uniqueness
  if (await this.checkNameExists(request.name)) {
    throw new DuplicateRepositoryNameError(request.name)
  }

  // 3. Create local repository config
  const repository: LocalRepository = {
    id: randomUUID(),
    kind: 'local',
    path: request.path,
    useGitWorktrees: request.useGitWorktrees ?? false,
    name: request.name,
    description: request.description,
    // ... other fields
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  // 4. Merge with .moatless/config.json if enabled
  if (request.syncWithRepoConfig) {
    await mergeWithRepoConfig(repository)
  }

  await this.saveRepository(repository)
  return repository
}

async createRemoteRepository(
  request: CreateRemoteRepositoryRequest,
  sessionToken?: string
): Promise<RemoteRepository> {
  // 1. Validate URL format based on provider
  await this.gitService.validateRemoteUrl(request.remoteUrl, request.provider)

  // 2. Check name uniqueness
  if (await this.checkNameExists(request.name)) {
    throw new DuplicateRepositoryNameError(request.name)
  }

  // 3. Verify repository exists using provider API
  const defaultBranch = await this.getProviderClient(request.provider)
    .getDefaultBranch(request.remoteUrl, sessionToken)

  // 4. Create remote repository config (NO cloning)
  const repository: RemoteRepository = {
    id: randomUUID(),
    kind: 'remote',
    remote: {
      url: request.remoteUrl,
      provider: request.provider,
      defaultBranch,
    },
    name: request.name,
    description: request.description,
    // ... other fields
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  await this.saveRepository(repository)
  return repository
}

async createRepositoryFromTemplate(
  repoName: string,
  templateId: string
): Promise<TemplateRepository> {
  // 1. Load template configuration
  const template = TEMPLATES.find(t => t.id === templateId)
  if (!template) {
    throw new ValidationError(`Template ${templateId} not found`)
  }

  // 2. Generate target path
  const repoDir = this.getRepositoriesDirectory()
  const targetPath = path.join(repoDir, 'templates', repoName)

  // 3. Clone template to target path
  await cloneRepository(template.gitUrl, targetPath)

  // 4. Disconnect from template origin
  await this.executeCommand('git remote remove origin', targetPath)

  // 5. Add marker commit
  await this.executeCommand(
    `git commit --allow-empty -m "Created from template: ${template.name}"`,
    targetPath
  )

  // 6. Create template repository config
  const repository: TemplateRepository = {
    id: randomUUID(),
    kind: 'template',
    templateId,
    path: targetPath,
    name: repoName,
    description: `Created from template: ${template.name}`,
    useGitWorktrees: false,
    systemPrompt: template.systemPrompt,
    codeExecution: {
      setupCommands: template.setupCommand ? [template.setupCommand] : undefined,
    },
    previewConfig: {
      ...template.previewConfig,
      takeScreenshots: true,
      enableInspector: true,
    },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  await this.saveRepository(repository)
  return repository
}
```

#### Branch Operations

```typescript
async getBranches(repository: RepositoryConfig): Promise<string[]> {
  switch (repository.kind) {
    case 'local':
    case 'template':
      // Use local git commands
      return await this.gitService.getBranches(repository.path)

    case 'remote':
      // Use provider API (no local clone needed)
      return await this.getProviderClient(repository.remote.provider)
        .listBranches(repository.remote.url)
  }
}

private getProviderClient(provider: string): GitProviderClient {
  switch (provider) {
    case 'github':
      return this.githubClient
    case 'gitlab':
      return this.gitlabClient
    default:
      throw new Error(`Unknown provider: ${provider}`)
  }
}
```

### Git Provider Abstraction

**File:** `apps/backend/src/infrastructure/clients/git-provider.interface.ts`

```typescript
export interface GitProviderClient {
  /**
   * List branches for a remote repository
   */
  listBranches(remoteUrl: string, token?: string): Promise<string[]>

  /**
   * Get default branch name
   */
  getDefaultBranch(remoteUrl: string, token?: string): Promise<string>

  /**
   * Validate that repository exists and is accessible
   */
  validateRepository(remoteUrl: string, token?: string): Promise<void>

  /**
   * Parse repository URL to extract owner and repo name
   */
  parseUrl(remoteUrl: string): { owner: string; repo: string }
}
```

**GitHub Implementation:**
```typescript
export class GitHubProviderClient implements GitProviderClient {
  constructor(private octokit: Octokit) {}

  async listBranches(remoteUrl: string, token?: string): Promise<string[]> {
    const { owner, repo } = this.parseUrl(remoteUrl)
    const client = token ? new Octokit({ auth: token }) : this.octokit

    const { data: branches } = await client.rest.repos.listBranches({
      owner,
      repo,
      per_page: 100,
    })

    return branches.map(b => b.name)
  }

  async getDefaultBranch(remoteUrl: string, token?: string): Promise<string> {
    const { owner, repo } = this.parseUrl(remoteUrl)
    const client = token ? new Octokit({ auth: token }) : this.octokit

    const { data } = await client.rest.repos.get({ owner, repo })
    return data.default_branch
  }

  parseUrl(remoteUrl: string): { owner: string; repo: string } {
    const match = remoteUrl.match(/github\.com\/([^/]+)\/([^/.]+)/)
    if (!match) throw new Error('Invalid GitHub URL')
    return { owner: match[1], repo: match[2].replace('.git', '') }
  }
}
```

### Task Workspace Creation

**File:** `apps/backend/src/features/tasks/services/task-workspace.service.ts`

```typescript
async createWorkspace(
  repository: RepositoryConfig,
  task: Task
): Promise<Workspace> {
  switch (repository.kind) {
    case 'local':
    case 'template':
      if (repository.useGitWorktrees) {
        // Create worktree from repository.path
        return await this.createWorktreeWorkspace(repository, task)
      } else {
        // Use repository.path directly
        return { path: repository.path, mode: 'direct' }
      }

    case 'remote':
      // Always clone for remote repositories
      return await this.createCloneWorkspace(repository, task)
  }
}

private async createCloneWorkspace(
  repository: RemoteRepository,
  task: Task
): Promise<Workspace> {
  const workspaceDir = path.join(
    this.dataDir,
    'workspaces',
    'clones',
    `${repository.name}-clone-${task.id}`
  )

  // Clone from remote URL (no source path needed)
  await cloneRepository(repository.remote.url, workspaceDir, {
    branch: task.branch || repository.remote.defaultBranch,
    token: await this.getTokenForProvider(repository.remote.provider),
  })

  return { path: workspaceDir, mode: 'clone' }
}
```

### Migration Plan

#### Database Migration

**Migration File:** `apps/backend/src/db/migrations/XXXX_add_repository_kind.ts`

```typescript
export async function up(db: Database) {
  // Step 1: Add kind column with default
  await db.exec(`
    ALTER TABLE repositories
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'local'
  `)

  // Step 2: Backfill templates
  await db.exec(`
    UPDATE repositories
    SET kind = 'template'
    WHERE template_id IS NOT NULL
  `)

  // Step 3: Backfill remotes (had cloneOnNewTask + remoteUrl)
  await db.exec(`
    UPDATE repositories
    SET kind = 'remote',
        path = NULL,
        use_git_worktrees = NULL
    WHERE clone_on_new_task = 1
      AND remote_url IS NOT NULL
      AND template_id IS NULL
  `)

  // Step 4: Add provider column and backfill
  await db.exec(`
    ALTER TABLE repositories
    ADD COLUMN provider TEXT
  `)

  await db.exec(`
    UPDATE repositories
    SET provider = 'github'
    WHERE kind = 'remote'
  `)

  // Step 5: Remove obsolete columns
  await db.exec(`
    ALTER TABLE repositories
    DROP COLUMN clone_on_new_task
  `)

  // Step 6: Add vibe column to template_id for backward compat
  await db.exec(`
    ALTER TABLE repositories
    ADD COLUMN vibe INTEGER
  `)

  await db.exec(`
    UPDATE repositories
    SET vibe = 1
    WHERE kind = 'template'
  `)
}
```

#### Data Validation

After migration, validate:
```typescript
// Local repos must have path
assert(local.path !== null)
assert(local.remoteUrl === null)
assert(local.templateId === null)

// Remote repos must NOT have path
assert(remote.path === null)
assert(remote.remoteUrl !== null)
assert(remote.provider !== null)

// Template repos must have both
assert(template.path !== null)
assert(template.templateId !== null)
assert(template.remoteUrl === null)
```

### API & UI Changes

#### API Endpoints

**POST /api/repositories** - Create repository (discriminated by `kind`):
```typescript
// Request body (union type)
{
  kind: 'local',
  path: '/path/to/repo',
  name: 'my-repo',
  ...
}

// OR

{
  kind: 'remote',
  remoteUrl: 'https://github.com/owner/repo',
  provider: 'github',
  name: 'my-repo',
  ...
}

// OR

{
  kind: 'template',
  templateId: 'hello-world-ui',
  name: 'my-project',
  ...
}
```

**GET /api/repositories/:id/branches** - List branches:
- Local/Template: Run `git branch` on `repository.path`
- Remote: Call provider API (GitHub API, GitLab API)

#### UI Components

**Repository Creation Form:**
```tsx
// Radio buttons to select kind
<RadioGroup value={kind} onChange={setKind}>
  <Radio value="local">Local Repository</Radio>
  <Radio value="remote">Remote Repository (GitHub/GitLab)</Radio>
  <Radio value="template">From Template</Radio>
</RadioGroup>

{kind === 'local' && (
  <PathInput label="Repository Path" />
  <Checkbox label="Use Git Worktrees" />
)}

{kind === 'remote' && (
  <Input label="Remote URL" />
  <Select label="Provider">
    <Option value="github">GitHub</Option>
    <Option value="gitlab">GitLab</Option>
  </Select>
)}

{kind === 'template' && (
  <Select label="Template">
    {templates.map(t => <Option value={t.id}>{t.name}</Option>)}
  </Select>
)}
```

**Repository List:**
```tsx
// Show different badges based on kind
{repo.kind === 'local' && <Badge>Local</Badge>}
{repo.kind === 'remote' && (
  <Badge>Remote ({repo.remote.provider})</Badge>
)}
{repo.kind === 'template' && <Badge>Template</Badge>}
```

### Provider Support Roadmap

#### Phase 1: GitHub (Current)
- Implement `GitHubProviderClient`
- Support GitHub API for branch listing
- Use GitHub tokens for authentication

#### Phase 2: GitLab
- Implement `GitLabProviderClient`
- Support GitLab API for branch listing
- Add GitLab token management
- Update URL validation for `gitlab.com`

## Next Steps

1. **Schema Implementation:**
   - Update `repository.schemas.ts` with discriminated union
   - Add provider-specific URL validation
   - Create separate creation schemas for each kind

2. **Service Refactoring:**
   - Split `createRepository()` into three methods
   - Implement provider client interface
   - Update `getBranches()` to use provider APIs for remote repos

3. **Migration Execution:**
   - Write and test migration script
   - Backfill existing data with correct `kind` values
   - Validate data integrity post-migration

4. **Provider Clients:**
   - Implement `GitHubProviderClient`
   - Plan `GitLabProviderClient` structure

5. **Frontend Updates:**
   - Update repository creation UI with three modes
   - Handle remote repositories without path in components
   - Add provider-specific badges and icons
