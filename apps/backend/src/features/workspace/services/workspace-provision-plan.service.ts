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
        'Template repositories cannot be used directly for tasks. Create a repository from the template first.',
      )
    }

    const task = await this.taskRepository.get(taskId)
    const targetBranch = task?.targetBranch

    const plan: WorkspaceProvisionPlan = {
      mode: 'clone',
      branch: targetBranch,
      targetBranch,
      setupMetadataHash: this.computeSetupHash(repoConfig),
      remoteUrl: repoConfig.remote.url,
      provider: repoConfig.remote.provider,
      localTargetPath: this.getClonePath(repoConfig, taskId),
    }

    logger.debug(
      {
        taskId,
        mode: 'clone',
        branch: targetBranch || 'default',
        targetBranch: targetBranch || 'none',
      },
      'WorkspaceProvisionPlanService: Created provision plan',
    )

    return plan
  }

  resolveCloneUrl(_taskId: string, repoConfig: RepositoryConfig): Promise<string> {
    if (repoConfig.kind === 'remote') {
      return Promise.resolve(repoConfig.remote.url)
    }

    return Promise.reject(new Error('Template repositories cannot be used directly for tasks'))
  }

  private getClonePath(repoConfig: RepositoryConfig, taskId: string): string {
    if (repoConfig.kind === 'template') {
      throw new Error('Template repositories cannot be used directly for tasks')
    }

    const dataDir = getDataDir()
    const repoName = repoConfig.name
    const cloneDir = path.join(dataDir, 'workspaces', 'clones')
    return path.join(cloneDir, `${repoName}-clone-${taskId}`)
  }

  private computeSetupHash(repoConfig: RepositoryConfig): string {
    if (repoConfig.kind === 'template') {
      throw new Error('Template repositories cannot be used directly for tasks')
    }

    const metadata = {
      kind: repoConfig.kind,
      remoteUrl: repoConfig.remote.url,
      codeExecution: repoConfig.codeExecution,
    }

    return crypto.createHash('sha256').update(JSON.stringify(metadata)).digest('hex').slice(0, 16)
  }
}
