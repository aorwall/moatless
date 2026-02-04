import crypto from 'node:crypto'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { GitProviderTokenService } from '../../auth/services/git-provider-token.service'
import { ClaudeTokenService } from '../../claude/services/claude-token.service'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'

/**
 * ProvisionPlanFactory - Creates workspace provision plans
 *
 * Encapsulates all resolution logic for building a WorkspaceProvisionPlan:
 * - Repository configuration lookup
 * - Git provider token resolution
 * - Claude authentication resolution
 * - Task branch resolution
 * - Setup metadata hash computation
 */
@injectable()
export class ProvisionPlanFactory {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
    @inject(GitProviderTokenService)
    private readonly gitProviderTokenService: GitProviderTokenService,
    @inject(ClaudeTokenService) private readonly claudeTokenService: ClaudeTokenService,
  ) {}

  async createPlan(taskId: string, repositoryId: string): Promise<WorkspaceProvisionPlan> {
    const repoConfig = await this.resolveRepository(repositoryId)
    const gitCloneUrl = repoConfig.remote.url
    const gitProviderToken = await this.resolveGitProviderToken(gitCloneUrl)
    const { claudeOAuthToken, anthropicApiKey } = await this.resolveClaudeAuth()
    const gitBranch = await this.resolveTaskBranch(taskId)
    const setupMetadataHash = this.computeSetupHash(repoConfig)

    const plan: WorkspaceProvisionPlan = {
      remoteUrl: gitCloneUrl,
      provider: repoConfig.remote.provider,
      pushEnabled: repoConfig.kind !== 'template',
      setupMetadataHash,
      repoConfig,
      gitCloneUrl,
      gitBranch,
      gitProviderToken,
      claudeOAuthToken,
      anthropicApiKey,
    }

    logger.debug(
      {
        taskId,
        gitBranch: gitBranch || 'default',
        pushEnabled: plan.pushEnabled,
        isTemplateMode: repoConfig.kind === 'template',
      },
      'ProvisionPlanFactory: Created provision plan',
    )

    return plan
  }

  private async resolveRepository(repositoryId: string): Promise<RepositoryConfig> {
    const repoConfig = await this.repoService.getRepositoryById(repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }
    return repoConfig
  }

  private async resolveGitProviderToken(gitCloneUrl: string): Promise<string> {
    const gitProviderToken =
      await this.gitProviderTokenService.getTokenForGitOperations(gitCloneUrl)
    if (!gitProviderToken) {
      throw new Error(
        'Cannot provision workspace: GitHub token is required. ' +
          'Please connect a GitHub account (Settings → GitHub) or set GITHUB_PAT environment variable.',
      )
    }
    return gitProviderToken
  }

  private async resolveClaudeAuth(): Promise<{
    claudeOAuthToken: string | null
    anthropicApiKey: string | undefined
  }> {
    const anthropicApiKey = process.env['ANTHROPIC_API_KEY']
    let claudeOAuthToken = await this.claudeTokenService.getTokenForAgent()
    if (!claudeOAuthToken) {
      claudeOAuthToken = process.env['CLAUDE_CODE_OAUTH_TOKEN'] ?? null
    }

    const hasValidAnthropicKey = anthropicApiKey && anthropicApiKey !== 'placeholder'
    if (!(claudeOAuthToken || hasValidAnthropicKey)) {
      throw new Error(
        'Cannot create worker: No Claude authentication configured. ' +
          'Please connect a Claude account (Settings → Claude) or set ANTHROPIC_API_KEY.',
      )
    }

    return { claudeOAuthToken, anthropicApiKey }
  }

  private async resolveTaskBranch(taskId: string): Promise<string | undefined> {
    const task = await this.taskRepository.get(taskId)
    return task?.branch
  }

  private computeSetupHash(repoConfig: RepositoryConfig): string {
    const metadata = {
      kind: repoConfig.kind,
      remoteUrl: repoConfig.remote.url,
      codeExecution: repoConfig.codeExecution,
    }
    return crypto.createHash('sha256').update(JSON.stringify(metadata)).digest('hex').slice(0, 16)
  }
}
