import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { RepositoryConfig, Task } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { CONTAINER_WORKSPACE_PATH } from '../../workspace/types/workspace-runtime.types'

@injectable()
export class ClaudeService {
  /**
   * Get Claude projects directory path
   */
  public getClaudeProjectPath(): string {
    const claudeConfigDir = path.join(os.homedir(), '.claude')
    return path.join(claudeConfigDir, 'projects')
  }

  /**
   * Read Claude config.json file and return as object
   */
  public async readClaudeConfig(): Promise<Record<string, unknown>> {
    const claudeConfigDir = path.join(os.homedir(), '.claude')
    const configPath = path.join(claudeConfigDir, '.config.json')

    try {
      const configContent = await fs.readFile(configPath, 'utf-8')
      return JSON.parse(configContent)
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        throw new Error(`Claude config.json file does not exist on path ${configPath}`)
      }
      logger.error({ err: error as Error, configPath }, 'Error reading Claude config.json')
      throw error
    }
  }

  /**
   * Get all project directories from Claude config
   */
  public async getClaudeProjectDirectories(): Promise<string[]> {
    const config = await this.readClaudeConfig()

    if (!config['projects'] || typeof config['projects'] !== 'object') {
      logger.warn(
        {
          config,
        },
        'Claude config.json does not contain projects object',
      )
      return []
    }

    // Extract all keys from the projects object, which are the file paths
    return Object.keys(config['projects'])
  }

  /**
   * Generate project ID from workspace path
   * Uses the standard container workspace path since all containers use the same path
   */
  public generateProjectId(task: Task, _repository: RepositoryConfig): string {
    logger.info({ taskId: task.taskId }, 'Generating project ID')
    return CONTAINER_WORKSPACE_PATH.replace(/[^a-zA-Z0-9]/g, '-')
  }
}
