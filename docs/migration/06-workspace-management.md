# Workspace Management Migration Guide

This guide covers the migration of workspace management functionality from Python to TypeScript.

## Overview

Phase 6 focuses on migrating the workspace management system that handles local and Docker workspace creation, git worktree management, and workspace isolation for task execution.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/workspaces/workspace_manager.py` - Workspace management
- `/moatless-swarm/swarm/workspaces/base.py` - Base workspace class
- `/moatless-swarm/swarm/workspaces/local.py` - Local workspace implementation
- `/moatless-swarm/swarm/workspaces/docker.py` - Docker workspace implementation
- `/moatless-swarm/swarm/workspaces/server.py` - Server management

## Implementation

### 1. Workspace Service

#### server/services/workspace.ts
Port from: `swarm/workspaces/workspace_manager.py`

```typescript
import { exec } from 'child_process';
import { promisify } from 'util';
import { promises as fs } from 'fs';
import path from 'path';
import { randomUUID } from 'crypto';
import type { FastifyInstance } from 'fastify';
import type { Workspace, Environment, LocalWorkspace, DockerWorkspace } from '@shared/types';
import type { StorageBackend } from './storage.js';
import type { EnvironmentService } from './environment.js';
import type { EventService } from './event.js';
import { WorkspaceError, EnvironmentNotFoundError } from '@shared/constants/errors.js';
import { getCurrentBranch, createBranch, getGitDiffFiles } from '../utils/git-utils.js';

const execAsync = promisify(exec);

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map();
  
  constructor(
    private storage: StorageBackend,
    private environmentService: EnvironmentService,
    private eventService: EventService
  ) {}
  
  async createWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string
  ): Promise<Workspace> {
    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId);
    if (existing) return existing;
    
    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId);
    if (!environment) {
      throw new EnvironmentNotFoundError(environmentId);
    }
    
    // Create workspace based on code execution type
    const workspace = await this.createWorkspaceForType(
      taskId,
      environment,
      branchName
    );
    
    this.activeWorkspaces.set(taskId, workspace);
    await this.storage.saveWorkspace(workspace);
    
    return workspace;
  }
  
  async getWorkspace(taskId: string): Promise<Workspace | null> {
    // Check active workspaces first
    const active = this.activeWorkspaces.get(taskId);
    if (active) return active;
    
    // Load from storage
    const workspace = await this.storage.getWorkspace(taskId);
    if (workspace) {
      this.activeWorkspaces.set(taskId, workspace);
    }
    return workspace;
  }
  
  async listWorkspaces(): Promise<Workspace[]> {
    const workspaceKeys = await this.storage.list('workspaces/');
    const workspaces: Workspace[] = [];
    
    for (const key of workspaceKeys) {
      const workspaceId = key.replace('workspaces/', '');
      const workspace = await this.storage.getWorkspace(workspaceId);
      if (workspace) {
        workspaces.push(workspace);
        this.activeWorkspaces.set(workspaceId, workspace);
      }
    }
    
    return workspaces;
  }
  
  async updateWorkspaceStatus(
    taskId: string, 
    status: Workspace['status'], 
    error?: string
  ): Promise<void> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    workspace.status = status;
    if (error) {
      workspace.error = error;
    }
    
    await this.storage.saveWorkspace(workspace);
    this.activeWorkspaces.set(taskId, workspace);
  }
  
  async listFiles(
    taskId: string, 
    relativePath: string = '.', 
    recursive: boolean = false
  ): Promise<string[]> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    if (workspace.type === 'local') {
      return this.listLocalFiles(workspace as LocalWorkspace, relativePath, recursive);
    } else {
      return this.listDockerFiles(workspace as DockerWorkspace, relativePath, recursive);
    }
  }
  
  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    if (workspace.type === 'local') {
      return this.readLocalFile(workspace as LocalWorkspace, filePath);
    } else {
      return this.readDockerFile(workspace as DockerWorkspace, filePath);
    }
  }
  
  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    if (workspace.type === 'local') {
      await this.writeLocalFile(workspace as LocalWorkspace, filePath, content);
    } else {
      await this.writeDockerFile(workspace as DockerWorkspace, filePath, content);
    }
    
    // Emit file change event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'workspace',
      subType: 'file_changed',
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: { filePath, action: 'write' }
    });
  }
  
  async getGitDiff(taskId: string): Promise<any> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    if (workspace.type === 'local') {
      const localWorkspace = workspace as LocalWorkspace;
      return getGitDiffFiles(localWorkspace.path);
    } else {
      // For Docker workspaces, we'd need to execute git commands inside the container
      throw new WorkspaceError('Git diff not supported for Docker workspaces yet', taskId);
    }
  }
  
  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) return;
    
    await this.emitSetupEvent(taskId, 'cleanup_started');
    
    try {
      if (workspace.type === 'local') {
        await this.cleanupLocalWorkspace(workspace as LocalWorkspace);
      } else if (workspace.type === 'docker') {
        await this.cleanupDockerWorkspace(workspace as DockerWorkspace);
      }
      
      // Remove from active workspaces and storage
      this.activeWorkspaces.delete(taskId);
      await this.storage.delete(`workspaces/${taskId}`);
      
      await this.emitSetupEvent(taskId, 'cleanup_completed');
    } catch (error) {
      await this.emitSetupEvent(taskId, 'cleanup_failed', {
        error: error instanceof Error ? error.message : String(error)
      });
      throw error;
    }
  }
  
  private async createWorkspaceForType(
    taskId: string,
    environment: Environment,
    branchName?: string
  ): Promise<Workspace> {
    if (environment.codeExecution.type === 'docker') {
      return this.createDockerWorkspace(taskId, environment, branchName);
    } else {
      return this.createLocalWorkspace(taskId, environment, branchName);
    }
  }
  
  private async createLocalWorkspace(
    taskId: string,
    environment: Environment,
    branchName?: string
  ): Promise<LocalWorkspace> {
    await this.emitSetupEvent(taskId, 'setup_started');
    
    const workspace: LocalWorkspace = {
      id: taskId,
      type: 'local',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      path: ''
    };
    
    try {
      if (environment.repository.type === 'local') {
        // Create git worktree for local repositories
        const repoPath = environment.repository.path;
        const worktreePath = path.join(
          process.env.WORKTREE_DIR || './worktrees',
          taskId
        );
        
        // Get current branch to restore later
        const originalBranch = await getCurrentBranch(repoPath);
        
        // Create unique branch name
        const branch = branchName || `task-${taskId}-${Date.now()}`;
        
        // Create worktree with new branch
        await execAsync(
          `git worktree add "${worktreePath}" -b "${branch}"`,
          { 
            cwd: repoPath,
            timeout: 30000 
          }
        );
        
        workspace.path = worktreePath;
        workspace.branch = branch;
        workspace.originalBranch = originalBranch;
        workspace.status = 'ready';
        
        await this.emitSetupEvent(taskId, 'worktree_created', {
          path: worktreePath,
          branch
        });
      } else {
        // For remote repositories, clone to a local directory
        const clonePath = path.join(
          process.env.CLONE_DIR || './clones',
          taskId
        );
        
        await fs.mkdir(path.dirname(clonePath), { recursive: true });
        
        const cloneCommand = `git clone "${environment.repository.url}" "${clonePath}"`;
        await execAsync(cloneCommand, { timeout: 60000 });
        
        // Checkout specific branch if specified
        if (environment.repository.branch) {
          await execAsync(
            `git checkout "${environment.repository.branch}"`,
            { cwd: clonePath, timeout: 10000 }
          );
        }
        
        workspace.path = clonePath;
        workspace.branch = environment.repository.branch || 'main';
        workspace.status = 'ready';
        
        await this.emitSetupEvent(taskId, 'repository_cloned', {
          path: clonePath,
          url: environment.repository.url
        });
      }
      
      await this.emitSetupEvent(taskId, 'setup_completed');
      return workspace;
      
    } catch (error) {
      workspace.status = 'error';
      workspace.error = error instanceof Error ? error.message : String(error);
      
      await this.emitSetupEvent(taskId, 'setup_failed', {
        error: workspace.error
      });
      
      throw new WorkspaceError(
        `Failed to create local workspace: ${workspace.error}`,
        taskId
      );
    }
  }
  
  private async createDockerWorkspace(
    taskId: string,
    environment: Environment,
    branchName?: string
  ): Promise<DockerWorkspace> {
    await this.emitSetupEvent(taskId, 'setup_started');
    
    const workspace: DockerWorkspace = {
      id: taskId,
      type: 'docker',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      containerId: ''
    };
    
    try {
      const config = environment.codeExecution.config || {};
      const image = config.image || 'node:18-alpine';
      const workdir = config.workdir || '/workspace';
      
      // Create container
      const containerName = `moatless-task-${taskId}`;
      
      let createCommand = `docker create --name "${containerName}" -w "${workdir}"`;
      
      // Add volume mounts if specified
      if (config.volumes) {
        for (const volume of config.volumes) {
          createCommand += ` -v "${volume}"`;
        }
      }
      
      // Add environment variables
      if (config.env) {
        for (const [key, value] of Object.entries(config.env)) {
          createCommand += ` -e "${key}=${value}"`;
        }
      }
      
      createCommand += ` "${image}" sleep infinity`;
      
      const { stdout } = await execAsync(createCommand, { timeout: 30000 });
      const containerId = stdout.trim();
      
      // Start container
      await execAsync(`docker start "${containerId}"`, { timeout: 10000 });
      
      // Clone repository inside container if it's a remote repository
      if (environment.repository.type !== 'local') {
        const cloneCommand = `docker exec "${containerId}" git clone "${environment.repository.url}" .`;
        await execAsync(cloneCommand, { timeout: 60000 });
        
        if (environment.repository.branch) {
          const checkoutCommand = `docker exec "${containerId}" git checkout "${environment.repository.branch}"`;
          await execAsync(checkoutCommand, { timeout: 10000 });
        }
      }
      
      workspace.containerId = containerId;
      workspace.image = image;
      workspace.status = 'ready';
      
      await this.emitSetupEvent(taskId, 'container_created', {
        containerId,
        image
      });
      
      await this.emitSetupEvent(taskId, 'setup_completed');
      return workspace;
      
    } catch (error) {
      workspace.status = 'error';
      workspace.error = error instanceof Error ? error.message : String(error);
      
      await this.emitSetupEvent(taskId, 'setup_failed', {
        error: workspace.error
      });
      
      throw new WorkspaceError(
        `Failed to create Docker workspace: ${workspace.error}`,
        taskId
      );
    }
  }
  
  private async listLocalFiles(
    workspace: LocalWorkspace,
    relativePath: string,
    recursive: boolean
  ): Promise<string[]> {
    const fullPath = path.join(workspace.path, relativePath);
    
    try {
      const stats = await fs.stat(fullPath);
      if (stats.isFile()) {
        return [relativePath];
      }
      
      const entries = await fs.readdir(fullPath, { withFileTypes: true });
      const files: string[] = [];
      
      for (const entry of entries) {
        const entryPath = path.join(relativePath, entry.name);
        
        if (entry.isFile()) {
          files.push(entryPath);
        } else if (entry.isDirectory() && recursive && !entry.name.startsWith('.')) {
          const subFiles = await this.listLocalFiles(workspace, entryPath, recursive);
          files.push(...subFiles);
        }
      }
      
      return files.sort();
    } catch (error) {
      throw new WorkspaceError(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async listDockerFiles(
    workspace: DockerWorkspace,
    relativePath: string,
    recursive: boolean
  ): Promise<string[]> {
    try {
      const command = recursive 
        ? `docker exec "${workspace.containerId}" find "${relativePath}" -type f`
        : `docker exec "${workspace.containerId}" ls -1 "${relativePath}"`;
      
      const { stdout } = await execAsync(command, { timeout: 10000 });
      return stdout.trim().split('\n').filter(line => line.trim());
    } catch (error) {
      throw new WorkspaceError(
        `Failed to list Docker files: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async readLocalFile(workspace: LocalWorkspace, filePath: string): Promise<string> {
    const fullPath = path.join(workspace.path, filePath);
    
    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath);
    const resolvedWorkspace = path.resolve(workspace.path);
    
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new WorkspaceError(`Access denied: path outside workspace`, workspace.id);
    }
    
    try {
      return await fs.readFile(fullPath, 'utf-8');
    } catch (error) {
      throw new WorkspaceError(
        `Failed to read file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async readDockerFile(workspace: DockerWorkspace, filePath: string): Promise<string> {
    try {
      const command = `docker exec "${workspace.containerId}" cat "${filePath}"`;
      const { stdout } = await execAsync(command, { timeout: 10000 });
      return stdout;
    } catch (error) {
      throw new WorkspaceError(
        `Failed to read Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async writeLocalFile(
    workspace: LocalWorkspace, 
    filePath: string, 
    content: string
  ): Promise<void> {
    const fullPath = path.join(workspace.path, filePath);
    
    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath);
    const resolvedWorkspace = path.resolve(workspace.path);
    
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new WorkspaceError(`Access denied: path outside workspace`, workspace.id);
    }
    
    try {
      await fs.mkdir(path.dirname(fullPath), { recursive: true });
      await fs.writeFile(fullPath, content, 'utf-8');
    } catch (error) {
      throw new WorkspaceError(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async writeDockerFile(
    workspace: DockerWorkspace, 
    filePath: string, 
    content: string
  ): Promise<void> {
    try {
      // Create directory if needed
      const dirPath = path.dirname(filePath);
      if (dirPath !== '.') {
        await execAsync(
          `docker exec "${workspace.containerId}" mkdir -p "${dirPath}"`,
          { timeout: 5000 }
        );
      }
      
      // Write file using docker exec with tee
      const command = `docker exec -i "${workspace.containerId}" tee "${filePath}"`;
      const child = exec(command, { timeout: 10000 });
      
      child.stdin?.write(content);
      child.stdin?.end();
      
      await new Promise((resolve, reject) => {
        child.on('close', (code) => {
          if (code === 0) resolve(undefined);
          else reject(new Error(`Command failed with code ${code}`));
        });
        child.on('error', reject);
      });
    } catch (error) {
      throw new WorkspaceError(
        `Failed to write Docker file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async cleanupLocalWorkspace(workspace: LocalWorkspace): Promise<void> {
    if (!workspace.path) return;
    
    try {
      // If this was a worktree, remove it
      if (workspace.originalBranch) {
        const environment = await this.environmentService.getEnvironment(workspace.environmentId);
        if (environment?.repository.type === 'local') {
          const repoPath = environment.repository.path;
          
          // Remove worktree
          await execAsync(
            `git worktree remove "${workspace.path}" --force`,
            { 
              cwd: repoPath,
              timeout: 10000 
            }
          );
          
          // Delete the branch if it was created for this task
          if (workspace.branch?.startsWith('task-')) {
            try {
              await execAsync(
                `git branch -D "${workspace.branch}"`,
                { 
                  cwd: repoPath,
                  timeout: 5000 
                }
              );
            } catch {
              // Branch deletion is best effort
            }
          }
        }
      } else {
        // This was a clone, just remove the directory
        await fs.rm(workspace.path, { recursive: true, force: true });
      }
    } catch (error) {
      throw new WorkspaceError(
        `Failed to cleanup local workspace: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async cleanupDockerWorkspace(workspace: DockerWorkspace): Promise<void> {
    if (!workspace.containerId) return;
    
    try {
      // Stop container
      await execAsync(`docker stop "${workspace.containerId}"`, { timeout: 10000 });
      
      // Remove container
      await execAsync(`docker rm "${workspace.containerId}"`, { timeout: 10000 });
    } catch (error) {
      throw new WorkspaceError(
        `Failed to cleanup Docker workspace: ${error instanceof Error ? error.message : String(error)}`,
        workspace.id
      );
    }
  }
  
  private async emitSetupEvent(
    taskId: string, 
    subType: string, 
    payload: Record<string, any> = {}
  ): Promise<void> {
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'workspace',
      subType,
      taskId,
      timestamp: new Date(),
      isError: subType.includes('failed'),
      payload
    });
  }
}

export function createWorkspaceService(
  app: FastifyInstance,
  storage: StorageBackend,
  environmentService: EnvironmentService,
  eventService: EventService
): WorkspaceService {
  return new WorkspaceService(storage, environmentService, eventService);
}
```

### 2. Git Utilities

#### server/utils/git-utils.ts
Port from: `swarm/utils/git_utils.py`

```typescript
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

export interface GitDiffFile {
  status: string;
  filename: string;
  additions?: number;
  deletions?: number;
}

export interface GitDiffResponse {
  files: GitDiffFile[];
  totalChanges: number;
  totalAdditions: number;
  totalDeletions: number;
}

export async function getGitDiffFiles(repoPath: string): Promise<GitDiffResponse> {
  try {
    // Get file status changes
    const { stdout: statusOutput } = await execAsync('git diff --name-status', {
      cwd: repoPath,
      timeout: 10000
    });
    
    // Get detailed diff stats
    const { stdout: statOutput } = await execAsync('git diff --numstat', {
      cwd: repoPath,
      timeout: 10000
    });
    
    const statusLines = statusOutput.trim().split('\n').filter(line => line.trim());
    const statLines = statOutput.trim().split('\n').filter(line => line.trim());
    
    const files: GitDiffFile[] = [];
    let totalAdditions = 0;
    let totalDeletions = 0;
    
    // Parse status changes
    const fileStats = new Map<string, { additions: number; deletions: number }>();
    for (const statLine of statLines) {
      const [additions, deletions, filename] = statLine.split('\t');
      if (filename) {
        const add = additions === '-' ? 0 : parseInt(additions, 10);
        const del = deletions === '-' ? 0 : parseInt(deletions, 10);
        fileStats.set(filename, { additions: add, deletions: del });
        totalAdditions += add;
        totalDeletions += del;
      }
    }
    
    // Combine status and stats
    for (const statusLine of statusLines) {
      const [status, filename] = statusLine.split('\t');
      if (filename) {
        const stats = fileStats.get(filename) || { additions: 0, deletions: 0 };
        files.push({
          status,
          filename,
          additions: stats.additions,
          deletions: stats.deletions
        });
      }
    }
    
    return {
      files,
      totalChanges: files.length,
      totalAdditions,
      totalDeletions
    };
  } catch (error) {
    throw new Error(`Git diff failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getCurrentBranch(repoPath: string): Promise<string> {
  try {
    const { stdout } = await execAsync('git branch --show-current', {
      cwd: repoPath,
      timeout: 5000
    });
    return stdout.trim();
  } catch (error) {
    throw new Error(`Failed to get current branch: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function createBranch(repoPath: string, branchName: string): Promise<void> {
  try {
    await execAsync(`git checkout -b "${branchName}"`, {
      cwd: repoPath,
      timeout: 10000
    });
  } catch (error) {
    throw new Error(`Failed to create branch: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function switchBranch(repoPath: string, branchName: string): Promise<void> {
  try {
    await execAsync(`git checkout "${branchName}"`, {
      cwd: repoPath,
      timeout: 10000
    });
  } catch (error) {
    throw new Error(`Failed to switch branch: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getCommitHash(repoPath: string, short: boolean = true): Promise<string> {
  try {
    const command = short ? 'git rev-parse --short HEAD' : 'git rev-parse HEAD';
    const { stdout } = await execAsync(command, {
      cwd: repoPath,
      timeout: 5000
    });
    return stdout.trim();
  } catch (error) {
    throw new Error(`Failed to get commit hash: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getBranchList(repoPath: string): Promise<string[]> {
  try {
    const { stdout } = await execAsync('git branch --list', {
      cwd: repoPath,
      timeout: 5000
    });
    
    return stdout
      .split('\n')
      .map(line => line.replace(/^\*?\s+/, '').trim())
      .filter(branch => branch && !branch.startsWith('('));
  } catch (error) {
    throw new Error(`Failed to get branch list: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getGitStatus(repoPath: string): Promise<{
  clean: boolean;
  modified: string[];
  untracked: string[];
  staged: string[];
}> {
  try {
    const { stdout } = await execAsync('git status --porcelain', {
      cwd: repoPath,
      timeout: 5000
    });
    
    const lines = stdout.trim().split('\n').filter(line => line.trim());
    const modified: string[] = [];
    const untracked: string[] = [];
    const staged: string[] = [];
    
    for (const line of lines) {
      const status = line.substring(0, 2);
      const filename = line.substring(3);
      
      if (status[0] !== ' ' && status[0] !== '?') {
        staged.push(filename);
      }
      if (status[1] === 'M' || status[1] === 'D') {
        modified.push(filename);
      }
      if (status === '??') {
        untracked.push(filename);
      }
    }
    
    return {
      clean: lines.length === 0,
      modified,
      untracked,
      staged
    };
  } catch (error) {
    throw new Error(`Failed to get git status: ${error instanceof Error ? error.message : String(error)}`);
  }
}
```

### 3. Docker Utilities

#### server/utils/docker-utils.ts
```typescript
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

export interface DockerContainer {
  id: string;
  name: string;
  image: string;
  status: string;
  created: Date;
}

export interface DockerImage {
  id: string;
  repository: string;
  tag: string;
  size: string;
  created: Date;
}

export async function listContainers(all: boolean = false): Promise<DockerContainer[]> {
  try {
    const command = all ? 'docker ps -a --format json' : 'docker ps --format json';
    const { stdout } = await execAsync(command, { timeout: 10000 });
    
    if (!stdout.trim()) return [];
    
    const lines = stdout.trim().split('\n');
    return lines.map(line => {
      const container = JSON.parse(line);
      return {
        id: container.ID,
        name: container.Names,
        image: container.Image,
        status: container.Status,
        created: new Date(container.CreatedAt)
      };
    });
  } catch (error) {
    throw new Error(`Failed to list containers: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function containerExists(nameOrId: string): Promise<boolean> {
  try {
    await execAsync(`docker inspect "${nameOrId}"`, { timeout: 5000 });
    return true;
  } catch {
    return false;
  }
}

export async function isContainerRunning(nameOrId: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync(
      `docker inspect -f '{{.State.Running}}' "${nameOrId}"`,
      { timeout: 5000 }
    );
    return stdout.trim() === 'true';
  } catch {
    return false;
  }
}

export async function removeContainer(nameOrId: string, force: boolean = false): Promise<void> {
  try {
    const command = force ? `docker rm -f "${nameOrId}"` : `docker rm "${nameOrId}"`;
    await execAsync(command, { timeout: 10000 });
  } catch (error) {
    throw new Error(`Failed to remove container: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function executeInContainer(
  nameOrId: string,
  command: string,
  options: {
    workdir?: string;
    user?: string;
    env?: Record<string, string>;
    timeout?: number;
  } = {}
): Promise<{ stdout: string; stderr: string }> {
  try {
    let dockerCommand = `docker exec`;
    
    if (options.workdir) {
      dockerCommand += ` -w "${options.workdir}"`;
    }
    
    if (options.user) {
      dockerCommand += ` -u "${options.user}"`;
    }
    
    if (options.env) {
      for (const [key, value] of Object.entries(options.env)) {
        dockerCommand += ` -e "${key}=${value}"`;
      }
    }
    
    dockerCommand += ` "${nameOrId}" ${command}`;
    
    const { stdout, stderr } = await execAsync(dockerCommand, {
      timeout: options.timeout || 30000
    });
    
    return { stdout, stderr };
  } catch (error) {
    throw new Error(`Failed to execute in container: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function copyToContainer(
  nameOrId: string,
  hostPath: string,
  containerPath: string
): Promise<void> {
  try {
    await execAsync(`docker cp "${hostPath}" "${nameOrId}:${containerPath}"`, {
      timeout: 30000
    });
  } catch (error) {
    throw new Error(`Failed to copy to container: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function copyFromContainer(
  nameOrId: string,
  containerPath: string,
  hostPath: string
): Promise<void> {
  try {
    await execAsync(`docker cp "${nameOrId}:${containerPath}" "${hostPath}"`, {
      timeout: 30000
    });
  } catch (error) {
    throw new Error(`Failed to copy from container: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getContainerLogs(
  nameOrId: string,
  options: {
    tail?: number;
    since?: string;
    timestamps?: boolean;
  } = {}
): Promise<string> {
  try {
    let command = `docker logs "${nameOrId}"`;
    
    if (options.tail) {
      command += ` --tail ${options.tail}`;
    }
    
    if (options.since) {
      command += ` --since "${options.since}"`;
    }
    
    if (options.timestamps) {
      command += ' --timestamps';
    }
    
    const { stdout } = await execAsync(command, { timeout: 10000 });
    return stdout;
  } catch (error) {
    throw new Error(`Failed to get container logs: ${error instanceof Error ? error.message : String(error)}`);
  }
}
```

## Testing Strategy

### Unit Tests

#### server/services/__tests__/workspace.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { WorkspaceService } from '../workspace.js';
import { FileStorage } from '../storage.js';
import { EventService } from '../event.js';
import { EnvironmentService } from '../environment.js';
import { promises as fs } from 'fs';
import path from 'path';

describe('WorkspaceService', () => {
  let service: WorkspaceService;
  let storage: FileStorage;
  let eventService: EventService;
  let environmentService: EnvironmentService;
  let tempDir: string;
  
  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now());
    await fs.mkdir(tempDir, { recursive: true });
    
    storage = new FileStorage(tempDir);
    eventService = new EventService(storage);
    environmentService = new EnvironmentService(storage);
    service = new WorkspaceService(storage, environmentService, eventService);
  });
  
  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });
  
  describe('createWorkspace', () => {
    it('should create a local workspace', async () => {
      // Create test environment
      const repoPath = path.join(tempDir, 'test-repo');
      await fs.mkdir(repoPath, { recursive: true });
      await fs.mkdir(path.join(repoPath, '.git'), { recursive: true });
      
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: repoPath
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const workspace = await service.createWorkspace('test-task', environment.id);
      
      expect(workspace.id).toBe('test-task');
      expect(workspace.type).toBe('local');
      expect(workspace.status).toBe('ready');
      expect(workspace.path).toBeDefined();
    });
    
    it('should handle workspace creation failures', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Invalid Environment',
        repository: {
          type: 'local',
          path: '/non/existent/path'
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      await expect(service.createWorkspace('test-task', environment.id))
        .rejects.toThrow();
    });
  });
  
  describe('file operations', () => {
    it('should read and write files', async () => {
      // Set up workspace
      const repoPath = path.join(tempDir, 'test-repo');
      await fs.mkdir(repoPath, { recursive: true });
      await fs.mkdir(path.join(repoPath, '.git'), { recursive: true });
      
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: repoPath
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const workspace = await service.createWorkspace('test-task', environment.id);
      
      // Write file
      const content = 'console.log("Hello, world!");';
      await service.writeFile('test-task', 'test.js', content);
      
      // Read file
      const readContent = await service.readFile('test-task', 'test.js');
      expect(readContent).toBe(content);
      
      // List files
      const files = await service.listFiles('test-task');
      expect(files).toContain('test.js');
    });
    
    it('should prevent path traversal attacks', async () => {
      const repoPath = path.join(tempDir, 'test-repo');
      await fs.mkdir(repoPath, { recursive: true });
      await fs.mkdir(path.join(repoPath, '.git'), { recursive: true });
      
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: repoPath
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      await service.createWorkspace('test-task', environment.id);
      
      // Attempt path traversal
      await expect(service.readFile('test-task', '../../../etc/passwd'))
        .rejects.toThrow('Access denied');
    });
  });
});
```

## Migration Checklist

- [ ] Create workspace service with local and Docker support
- [ ] Implement git worktree management for local workspaces
- [ ] Add Docker container lifecycle management
- [ ] Create file operation utilities (read, write, list)
- [ ] Implement git utilities for diff and branch management
- [ ] Add workspace cleanup and isolation
- [ ] Create Docker utilities for container operations
- [ ] Add comprehensive error handling and validation
- [ ] Implement security measures (path traversal protection)
- [ ] Write unit tests for all workspace operations
- [ ] Add integration tests with real git repositories
- [ ] Test Docker workspace functionality
- [ ] Validate workspace isolation and security
- [ ] Test cleanup and resource management

## Security Considerations

### 1. Path Traversal Protection
- Validate all file paths are within workspace boundaries
- Use path.resolve() to normalize paths
- Reject paths containing '../' or absolute paths

### 2. Container Security
- Use specific Docker images, not 'latest'
- Limit container resources (memory, CPU)
- Run containers with non-root users when possible
- Clean up containers promptly

### 3. Git Operations
- Validate branch names and repository URLs
- Use timeouts for all git operations
- Sanitize git command inputs
- Handle authentication securely

## Performance Optimization

### 1. Workspace Management
- Reuse existing workspaces when possible
- Implement workspace caching
- Use background cleanup for better responsiveness
- Monitor disk usage and implement cleanup policies

### 2. File Operations
- Stream large files instead of loading into memory
- Implement file operation timeouts
- Cache frequently accessed files
- Use efficient directory traversal

### 3. Docker Operations
- Pull images asynchronously
- Reuse containers when possible
- Implement container pooling for common images
- Monitor container resource usage

## Next Steps

1. Review [Agent Integration Migration](./07-agent-integration.md)
2. Implement advanced workspace features
3. Add workspace monitoring and metrics
4. Set up automated workspace cleanup
5. Configure container orchestration policies