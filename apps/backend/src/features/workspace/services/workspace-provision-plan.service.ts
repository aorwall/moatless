import crypto from 'node:crypto'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { RepositoryService } from '../../repositories/index'
import { TaskRepository } from '../../tasks/db/task.repository'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'

@injectable()
export class WorkspaceProvisionPlanService {
  constructor(
    @inject(RepositoryService) private readonly repoService: RepositoryService,
    @inject(TaskRepository) private readonly taskRepository: TaskRepository,
  ) {}

  async getRepositoryConfig(repositoryId: string): Promise<RepositoryConfig> {
    const repoConfig = await this.repoService.getRepositoryById(repositoryId)
    if (!repoConfig) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }
    return repoConfig
  }

  async createProvisionPlan(
    taskId: string,
    repoConfig: RepositoryConfig,
  ): Promise<WorkspaceProvisionPlan> {
    if (repoConfig.kind === 'template') {
      throw new Error(
        'Template repositories cannot be used directly for tasks. Create a local repository from the template first.',
      )
    }

    const mode = this.determineProvisionMode(repoConfig)
    const task = await this.taskRepository.get(taskId)
    const targetBranch = task?.targetBranch
    const branch = targetBranch || (mode === 'worktree' ? `workspace-${taskId}` : undefined)

    const plan: WorkspaceProvisionPlan = {
      mode,
      branch,
      targetBranch,
      setupMetadataHash: this.computeSetupHash(repoConfig),
    }

    switch (mode) {
      case 'direct':
        if (repoConfig.kind === 'local') {
          plan.reusePath = repoConfig.path
        } else {
          throw new Error('Direct mode only supported for local repositories')
        }
        break

      case 'worktree':
        if (repoConfig.kind === 'local') {
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getWorktreePath(repoConfig, taskId)
        } else {
          throw new Error('Worktree mode only supported for local repositories')
        }
        break

      case 'clone':
        if (repoConfig.kind === 'remote') {
          plan.remoteUrl = repoConfig.remote.url
          plan.provider = repoConfig.remote.provider
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        } else if (repoConfig.kind === 'local') {
          plan.reusePath = repoConfig.path
          plan.localTargetPath = this.getClonePath(repoConfig, taskId)
        }
        break
    }

    logger.debug(
      {
        taskId,
        mode,
        branch: branch || 'default',
        targetBranch: targetBranch || 'none',
      },
      'WorkspaceProvisionPlanService: Created provision plan',
    )

    return plan
  }

  async resolveCloneUrl(taskId: string, repoConfig: RepositoryConfig): Promise<string> {
    if (repoConfig.kind === 'remote') {
      return repoConfig.remote.url
    }

    if (repoConfig.kind === 'local') {
      const task = await this.taskRepository.getInternal(taskId)
      if (task?.templateId) {
        const templateRepo = await this.repoService.getRepositoryById(task.templateId)
        if (templateRepo?.kind === 'template') {
          logger.info(
            { taskId, templateId: task.templateId, gitCloneUrl: templateRepo.remote.url },
            'WorkspaceProvisionPlanService: Using template remote URL for workspace clone',
          )
          return templateRepo.remote.url
        }
        throw new Error(
          `Template repository not found: ${task.templateId}. ` +
            'Local repositories without templates are not supported in Kubernetes mode.',
        )
      }
      throw new Error(
        'Kubernetes execution mode requires remote repository. ' +
          'Local repositories (without template) are not supported in Kubernetes mode.',
      )
    }

    throw new Error('Template repositories cannot be used directly for tasks')
  }

  private determineProvisionMode(repoConfig: RepositoryConfig): 'direct' | 'worktree' | 'clone' {
    switch (repoConfig.kind) {
      case 'remote':
        return 'clone'
      case 'local':
        return repoConfig.useGitWorktrees ? 'worktree' : 'direct'
      case 'template':
        throw new Error('Template repositories cannot be used directly for tasks')
      default: {
        const _exhaustiveCheck: never = repoConfig
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
    }
  }

  private getWorktreePath(repoConfig: RepositoryConfig, taskId: string): string {
    if (repoConfig.kind !== 'local') {
      throw new Error('Worktree path only supported for local repositories')
    }

    const dataDir = getDataDir()
    const repoName = path.basename(repoConfig.path)

    const worktreeDir = path.join(dataDir, 'workspaces', 'worktrees')
    return path.join(worktreeDir, `${repoName}-workspace-${taskId}`)
  }

  private getClonePath(repoConfig: RepositoryConfig, taskId: string): string {
    const dataDir = getDataDir()
    let repoName: string

    if (repoConfig.kind === 'local') {
      repoName = path.basename(repoConfig.path)
    } else if (repoConfig.kind === 'remote') {
      repoName = repoConfig.name
    } else if (repoConfig.kind === 'template') {
      throw new Error('Template repositories cannot be used directly for tasks')
    } else {
      const _exhaustiveCheck: never = repoConfig
      throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
    }

    const cloneDir = path.join(dataDir, 'workspaces', 'clones')
    return path.join(cloneDir, `${repoName}-clone-${taskId}`)
  }

  private computeSetupHash(repoConfig: RepositoryConfig): string {
    let metadata: Record<string, unknown>

    switch (repoConfig.kind) {
      case 'local':
        metadata = {
          kind: repoConfig.kind,
          path: repoConfig.path,
          codeExecution: repoConfig.codeExecution,
          useGitWorktrees: repoConfig.useGitWorktrees,
        }
        break

      case 'remote':
      case 'template':
        metadata = {
          kind: repoConfig.kind,
          remoteUrl: repoConfig.remote.url,
          codeExecution: repoConfig.codeExecution,
        }
        break

      default: {
        const _exhaustiveCheck: never = repoConfig
        throw new Error(`Unknown repository kind: ${(_exhaustiveCheck as RepositoryConfig).kind}`)
      }
    }

    return crypto.createHash('sha256').update(JSON.stringify(metadata)).digest('hex').slice(0, 16)
  }
}
