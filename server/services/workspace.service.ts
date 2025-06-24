import { EnvironmentNotFoundError, WorkspaceError } from '@shared/constants/errors.js'
import { exec } from 'child_process'
import { randomUUID } from 'crypto'
import type { FastifyInstance } from 'fastify'
import { promises as fs } from 'fs'
import path from 'path'
import { promisify } from 'util'
import type {
  DockerWorkspace,
  Environment,
  LocalWorkspace,
  Workspace,
} from '../../shared/types/index.js'
import { getCurrentBranch, getGitDiffFiles } from '../utils/git-utils.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'

const execAsync = promisify(exec)

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map()

  constructor(
    private storage: StorageBackend,
    private environmentService: EnvironmentService,
    private eventService: EventService,
  ) {}

  async createWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string,
  ): Promise<Workspace> {
    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId)
    if (existing) return existing

    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      throw new EnvironmentNotFoundError(environmentId)
    }

    // Create workspace based on code execution type
    const workspace = await this.createWorkspaceForType(taskId, environment, branchName)

    this.activeWorkspaces.set(taskId, workspace)
    await this.storage.saveWorkspace(workspace)

    return workspace
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    // Check active workspaces first
    const active = this.activeWorkspaces.get(taskId)
    if (active) return active

    // Load from storage
    const workspace = await this.storage.getWorkspace(taskId)
    if (workspace) {
      this.activeWorkspaces.set(taskId, workspace)
    }
    return workspace
  }

  async listWorkspaces(): Promise<Workspace[]> {
    const workspaceKeys = await this.storage.list('workspaces/')
    const workspaces: Workspace[] = []

    for (const key of workspaceKeys) {
      const workspaceId = key.replace('workspaces/', '')
      const workspace = await this.storage.getWorkspace(workspaceId)
      if (workspace) {
        workspaces.push(workspace)
        this.activeWorkspaces.set(workspaceId, workspace)
      }
    }

    return workspaces
  }

  async updateWorkspaceStatus(
    taskId: string,
    status: Workspace['status'],
    error?: string,
  ): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    workspace.status = status
    if (error) {
      workspace.error = error
    }

    await this.storage.saveWorkspace(workspace)
    this.activeWorkspaces.set(taskId, workspace)
  }

  async listFiles(
    taskId: string,
    relativePath: string = '.',
    recursive: boolean = false,
  ): Promise<string[]> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (workspace.type === 'local') {
      return this.listLocalFiles(workspace as LocalWorkspace, relativePath, recursive)
    } else {
      return this.listDockerFiles(workspace as DockerWorkspace, relativePath, recursive)
    }
  }

  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (workspace.type === 'local') {
      return this.readLocalFile(workspace as LocalWorkspace, filePath)
    } else {
      return this.readDockerFile(workspace as DockerWorkspace, filePath)
    }
  }

  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (workspace.type === 'local') {
      await this.writeLocalFile(workspace as LocalWorkspace, filePath, content)
    } else {
      await this.writeDockerFile(workspace as DockerWorkspace, filePath, content)
    }

    // Emit file change event
    await this.eventService.emit(
      randomUUID(),
      'workspace',
      taskId,
      new Date(),
      false,
      { filePath, action: 'write' },
      'file_changed',
    )
  }

  async getGitDiff(taskId: string): Promise<any> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (workspace.type === 'local') {
      const localWorkspace = workspace as LocalWorkspace
      return getGitDiffFiles(localWorkspace.path)
    } else {
      // For Docker workspaces, we'd need to execute git commands inside the container
      throw new WorkspaceError('Git diff not supported for Docker workspaces yet', taskId)
    }
  }

  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) return

    await this.emitSetupEvent(taskId, 'cleanup_started')

    try {
      if (workspace.type === 'local') {
        await this.cleanupLocalWorkspace(workspace as LocalWorkspace)
      } else if (workspace.type === 'docker') {
        await this.cleanupDockerWorkspace(workspace as DockerWorkspace)
      }

      // Remove from active workspaces and storage
      this.activeWorkspaces.delete(taskId)
      await this.storage.delete(`workspaces/${taskId}`)

      await this.emitSetupEvent(taskId, 'cleanup_completed')
    } catch (error) {
      await this.emitSetupEvent(taskId, 'cleanup_failed', {
        error: error instanceof Error ? error.message : String(error),
      })
      throw error
    }
  }

  private async createWorkspaceForType(
    taskId: string,
    environment: Environment,
    branchName?: string,
  ): Promise<Workspace> {
    if (environment.codeExecution.type === 'docker') {
      return this.createDockerWorkspace(taskId, environment, branchName)
    } else {
      return this.createLocalWorkspace(taskId, environment, branchName)
    }
  }

  private async createLocalWorkspace(
    taskId: string,
    environment: Environment,
    branchName?: string,
  ): Promise<LocalWorkspace> {
    await this.emitSetupEvent(taskId, 'setup_started')

    const workspace: LocalWorkspace = {
      id: taskId,
      type: 'local',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      path: '',
    }

    try {
      if (environment.repository.type === 'local') {
        // Create git worktree for local repositories
        const repoPath = environment.repository.path
        const worktreePath = path.join(process.env.WORKTREE_DIR || './worktrees', taskId)

        // Get current branch to restore later
        const originalBranch = await getCurrentBranch(repoPath)

        // Create unique branch name
        const branch = branchName || `task-${taskId}-${Date.now()}`

        // Create worktree with new branch
        await execAsync(`git worktree add "${worktreePath}" -b "${branch}"`, {
          cwd: repoPath,
          timeout: 30000,
        })

        workspace.path = worktreePath
        workspace.branch = branch
        workspace.originalBranch = originalBranch
        workspace.status = 'ready'

        await this.emitSetupEvent(taskId, 'worktree_created', {
          path: worktreePath,
          branch,
        })
      } else {
        // For remote repositories, clone to a local directory
        const clonePath = path.join(process.env.CLONE_DIR || './clones', taskId)

        await fs.mkdir(path.dirname(clonePath), { recursive: true })

        const cloneCommand = `git clone "${environment.repository.url}" "${clonePath}"`
        await execAsync(cloneCommand, { timeout: 60000 })

        // Checkout specific branch if specified
        if (environment.repository.branch) {
          await execAsync(`git checkout "${environment.repository.branch}"`, {
            cwd: clonePath,
            timeout: 10000,
          })
        }

        workspace.path = clonePath
        workspace.branch = environment.repository.branch || 'main'
        workspace.status = 'ready'

        await this.emitSetupEvent(taskId, 'repository_cloned', {
          path: clonePath,
          url: environment.repository.url,
        })
      }

      await this.emitSetupEvent(taskId, 'setup_completed')
      return workspace
    } catch (error) {
      workspace.status = 'error'
      workspace.error = error instanceof Error ? error.message : String(error)

      await this.emitSetupEvent(taskId, 'setup_failed', {
        error: workspace.error,
      })

      throw new WorkspaceError(`Failed to create local workspace: ${workspace.error}`, taskId)
    }
  }

  private async createDockerWorkspace(
    taskId: string,
    environment: Environment,
    _branchName?: string,
  ): Promise<DockerWorkspace> {
    await this.emitSetupEvent(taskId, 'setup_started')

    const workspace: DockerWorkspace = {
      id: taskId,
      type: 'docker',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      containerId: '',
    }

    try {
      const config = environment.codeExecution.config || {}
      const image = config.image || 'node:18-alpine'
      const workdir = config.workdir || '/workspace'

      // Create container
      const containerName = `moatless-task-${taskId}`

      let createCommand = `docker create --name "${containerName}" -w "${workdir}"`

      // Add volume mounts if specified
      if (config.volumes) {
        for (const volume of config.volumes) {
          createCommand += ` -v "${volume}"`
        }
      }

      // Add environment variables
      if (config.env) {
        for (const [key, value] of Object.entries(config.env)) {
          createCommand += ` -e "${key}=${value}"`
        }
      }

      createCommand += ` "${image}" sleep infinity`

      const { stdout } = await execAsync(createCommand, { timeout: 30000 })
      const containerId = stdout.trim()

      // Start container
      await execAsync(`docker start "${containerId}"`, { timeout: 10000 })

      // Clone repository inside container if it's a remote repository
      if (environment.repository.type !== 'local') {
        const cloneCommand = `docker exec "${containerId}" git clone "${environment.repository.url}" .`
        await execAsync(cloneCommand, { timeout: 60000 })

        if (environment.repository.branch) {
          const checkoutCommand = `docker exec "${containerId}" git checkout "${environment.repository.branch}"`
          await execAsync(checkoutCommand, { timeout: 10000 })
        }
      }

      workspace.containerId = containerId
      workspace.image = image
      workspace.status = 'ready'

      await this.emitSetupEvent(taskId, 'container_created', {
        containerId,
        image,
      })

      await this.emitSetupEvent(taskId, 'setup_completed')
      return workspace
    } catch (error) {
      workspace.status = 'error'
      workspace.error = error instanceof Error ? error.message : String(error)

      await this.emitSetupEvent(taskId, 'setup_failed', {
        error: workspace.error,
      })

      throw new WorkspaceError(`Failed to create Docker workspace: ${workspace.error}`, taskId)
    }
  }

  private async listLocalFiles(
    workspace: LocalWorkspace,
    relativePath: string,
    recursive: boolean,
  ): Promise<string[]> {
    const fullPath = path.join(workspace.path, relativePath)

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const entries = await fs.readdir(fullPath, { withFileTypes: true })
      const files: string[] = []

      for (const entry of entries) {
        const entryPath = path.join(relativePath, entry.name)

        if (entry.isFile()) {
          files.push(entryPath)
        } else if (entry.isDirectory() && recursive && !entry.name.startsWith('.')) {
          const subFiles = await this.listLocalFiles(workspace, entryPath, recursive)
          files.push(...subFiles)
        }
      }

      return files.sort()
    } catch (error) {
      throw new WorkspaceError(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async listDockerFiles(
    workspace: DockerWorkspace,
    relativePath: string,
    recursive: boolean,
  ): Promise<string[]> {
    try {
      const command = recursive
        ? `docker exec "${workspace.containerId}" find "${relativePath}" -type f`
        : `docker exec "${workspace.containerId}" ls -1 "${relativePath}"`

      const { stdout } = await execAsync(command, { timeout: 10000 })
      return stdout
        .trim()
        .split('\n')
        .filter((line) => line.trim())
    } catch (error) {
      throw new WorkspaceError(
        `Failed to list Docker files: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async readLocalFile(workspace: LocalWorkspace, filePath: string): Promise<string> {
    const fullPath = path.join(workspace.path, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(workspace.path)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new WorkspaceError(`Access denied: path outside workspace`, workspace.id)
    }

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      throw new WorkspaceError(
        `Failed to read file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async readDockerFile(workspace: DockerWorkspace, filePath: string): Promise<string> {
    try {
      const command = `docker exec "${workspace.containerId}" cat "${filePath}"`
      const { stdout } = await execAsync(command, { timeout: 10000 })
      return stdout
    } catch (error) {
      throw new WorkspaceError(
        `Failed to read Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async writeLocalFile(
    workspace: LocalWorkspace,
    filePath: string,
    content: string,
  ): Promise<void> {
    const fullPath = path.join(workspace.path, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(workspace.path)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new WorkspaceError(`Access denied: path outside workspace`, workspace.id)
    }

    try {
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')
    } catch (error) {
      throw new WorkspaceError(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async writeDockerFile(
    workspace: DockerWorkspace,
    filePath: string,
    content: string,
  ): Promise<void> {
    try {
      // Create directory if needed
      const dirPath = path.dirname(filePath)
      if (dirPath !== '.') {
        await execAsync(`docker exec "${workspace.containerId}" mkdir -p "${dirPath}"`, {
          timeout: 5000,
        })
      }

      // Write file using docker exec with tee
      const command = `docker exec -i "${workspace.containerId}" tee "${filePath}"`
      const child = exec(command, { timeout: 10000 })

      child.stdin?.write(content)
      child.stdin?.end()

      await new Promise((resolve, reject) => {
        child.on('close', (code) => {
          if (code === 0) resolve(undefined)
          else reject(new Error(`Command failed with code ${code}`))
        })
        child.on('error', reject)
      })
    } catch (error) {
      throw new WorkspaceError(
        `Failed to write Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async cleanupLocalWorkspace(workspace: LocalWorkspace): Promise<void> {
    if (!workspace.path) return

    try {
      // If this was a worktree, remove it
      if (workspace.originalBranch) {
        const environment = await this.environmentService.getEnvironment(workspace.environmentId)
        if (environment?.repository.type === 'local') {
          const repoPath = environment.repository.path

          // Remove worktree
          await execAsync(`git worktree remove "${workspace.path}" --force`, {
            cwd: repoPath,
            timeout: 10000,
          })

          // Delete the branch if it was created for this task
          if (workspace.branch?.startsWith('task-')) {
            try {
              await execAsync(`git branch -D "${workspace.branch}"`, {
                cwd: repoPath,
                timeout: 5000,
              })
            } catch {
              // Branch deletion is best effort
            }
          }
        }
      } else {
        // This was a clone, just remove the directory
        await fs.rm(workspace.path, { recursive: true, force: true })
      }
    } catch (error) {
      throw new WorkspaceError(
        `Failed to cleanup local workspace: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async cleanupDockerWorkspace(workspace: DockerWorkspace): Promise<void> {
    if (!workspace.containerId) return

    try {
      // Stop container
      await execAsync(`docker stop "${workspace.containerId}"`, { timeout: 10000 })

      // Remove container
      await execAsync(`docker rm "${workspace.containerId}"`, { timeout: 10000 })
    } catch (error) {
      throw new WorkspaceError(
        `Failed to cleanup Docker workspace: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id,
      )
    }
  }

  private async emitSetupEvent(
    taskId: string,
    subType: string,
    payload: Record<string, any> = {},
  ): Promise<void> {
    await this.eventService.emit(
      randomUUID(),
      'workspace',
      taskId,
      new Date(),
      subType.includes('failed'),
      payload,
      subType,
    )
  }
}

export function createWorkspaceService(
  _app: FastifyInstance,
  storage: StorageBackend,
  environmentService: EnvironmentService,
  eventService: EventService,
): WorkspaceService {
  return new WorkspaceService(storage, environmentService, eventService)
}
