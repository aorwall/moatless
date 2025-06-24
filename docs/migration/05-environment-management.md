# Environment Management Migration Guide

This guide covers the migration of environment management functionality from Python to TypeScript.

## Overview

Phase 5 focuses on migrating the environment management system that handles repository configuration, code execution environments, and file search capabilities.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/environments/environment_manager.py` - Environment management
- `/moatless-swarm/swarm/environments/environments.py` - Environment models
- `/moatless-swarm/swarm/environments/repository.py` - Repository handling
- `/moatless-swarm/swarm/utils/file_search.py` - File search utilities

## Implementation

### 1. Environment Service

#### server/services/environment.ts
Port from: `swarm/environments/environment_manager.py`

```typescript
import { randomUUID } from 'crypto';
import { promises as fs } from 'fs';
import path from 'path';
import type { FastifyInstance } from 'fastify';
import type { 
  Environment, 
  CreateEnvironmentRequest,
  FileSearchRequest, 
  FileSearchResponse,
  LocalRepository,
  RemoteRepository
} from '@shared/types';
import type { StorageBackend } from './storage.js';
import { EnvironmentNotFoundError, ValidationError } from '@shared/constants/errors.js';
import { searchFilesInDirectory } from '../utils/file-search.js';
import { validateRepositoryAccess } from '../utils/repository.js';

export class EnvironmentService {
  constructor(private storage: StorageBackend) {}
  
  async createEnvironment(request: CreateEnvironmentRequest): Promise<Environment> {
    // Validate repository configuration
    await this.validateRepository(request.repository);
    
    const environment: Environment = {
      id: randomUUID(),
      name: request.name,
      description: request.description,
      repository: request.repository,
      codeExecution: request.codeExecution,
      createdAt: new Date(),
      updatedAt: new Date()
    };
    
    await this.saveEnvironment(environment);
    return environment;
  }
  
  async saveEnvironment(env: Environment): Promise<void> {
    env.updatedAt = new Date();
    await this.storage.saveEnvironment(env);
  }
  
  async getEnvironment(envId: string): Promise<Environment | null> {
    return await this.storage.getEnvironment(envId);
  }
  
  async listEnvironments(): Promise<Environment[]> {
    const keys = await this.storage.list('environments/');
    const environments: Environment[] = [];
    
    for (const key of keys) {
      const envId = key.replace('environments/', '');
      const env = await this.storage.getEnvironment(envId);
      if (env) environments.push(env);
    }
    
    // Sort by creation date (newest first)
    return environments.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime());
  }
  
  async updateEnvironment(envId: string, updates: Partial<Environment>): Promise<Environment> {
    const environment = await this.getEnvironment(envId);
    if (!environment) {
      throw new EnvironmentNotFoundError(envId);
    }
    
    // Validate repository if being updated
    if (updates.repository) {
      await this.validateRepository(updates.repository);
    }
    
    const updatedEnvironment = {
      ...environment,
      ...updates,
      id: envId, // Ensure ID doesn't change
      updatedAt: new Date()
    };
    
    await this.saveEnvironment(updatedEnvironment);
    return updatedEnvironment;
  }
  
  async deleteEnvironment(envId: string): Promise<void> {
    const environment = await this.getEnvironment(envId);
    if (!environment) {
      throw new EnvironmentNotFoundError(envId);
    }
    
    // Check if environment is in use by any tasks
    const tasks = await this.storage.list('tasks/');
    for (const taskKey of tasks) {
      const taskId = taskKey.replace('tasks/', '');
      const task = await this.storage.getTask(taskId);
      if (task && task.environmentId === envId) {
        throw new ValidationError(
          `Cannot delete environment ${envId} - it is in use by task ${taskId}`,
          { environmentId: envId, taskId }
        );
      }
    }
    
    await this.storage.delete(`environments/${envId}`);
  }
  
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    const environment = await this.getEnvironment(request.environmentId);
    if (!environment) {
      throw new EnvironmentNotFoundError(request.environmentId);
    }
    
    // Only support file search for local repositories currently
    if (environment.repository.type !== 'local') {
      throw new ValidationError(
        'File search is only supported for local repositories',
        { repositoryType: environment.repository.type }
      );
    }
    
    const localRepo = environment.repository as LocalRepository;
    
    return await searchFilesInDirectory(
      localRepo.path,
      request.query,
      request.maxResults || 100,
      request.filePattern
    );
  }
  
  async validateEnvironment(envId: string): Promise<boolean> {
    const environment = await this.getEnvironment(envId);
    if (!environment) {
      return false;
    }
    
    try {
      await this.validateRepository(environment.repository);
      return true;
    } catch {
      return false;
    }
  }
  
  async getRepositoryInfo(envId: string): Promise<{
    type: string;
    accessible: boolean;
    details: any;
  }> {
    const environment = await this.getEnvironment(envId);
    if (!environment) {
      throw new EnvironmentNotFoundError(envId);
    }
    
    const repository = environment.repository;
    
    try {
      const details = await this.getRepositoryDetails(repository);
      return {
        type: repository.type,
        accessible: true,
        details
      };
    } catch (error) {
      return {
        type: repository.type,
        accessible: false,
        details: { error: error instanceof Error ? error.message : String(error) }
      };
    }
  }
  
  private async validateRepository(repository: any): Promise<void> {
    switch (repository.type) {
      case 'local':
        await this.validateLocalRepository(repository as LocalRepository);
        break;
      case 'github':
      case 'gitlab':
        await this.validateRemoteRepository(repository as RemoteRepository);
        break;
      default:
        throw new ValidationError(
          `Unsupported repository type: ${repository.type}`,
          { repositoryType: repository.type }
        );
    }
  }
  
  private async validateLocalRepository(repo: LocalRepository): Promise<void> {
    try {
      const stats = await fs.stat(repo.path);
      if (!stats.isDirectory()) {
        throw new ValidationError(`Path ${repo.path} is not a directory`);
      }
      
      // Check if it's a git repository
      const gitPath = path.join(repo.path, '.git');
      try {
        await fs.access(gitPath);
      } catch {
        throw new ValidationError(`Path ${repo.path} is not a git repository`);
      }
    } catch (error) {
      if (error instanceof ValidationError) {
        throw error;
      }
      throw new ValidationError(
        `Cannot access repository at ${repo.path}: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }
  
  private async validateRemoteRepository(repo: RemoteRepository): Promise<void> {
    // Validate URL format
    try {
      new URL(repo.url);
    } catch {
      throw new ValidationError(`Invalid repository URL: ${repo.url}`);
    }
    
    // Additional validation could include checking if repo is accessible
    // This would require implementing git operations or API calls
    await validateRepositoryAccess(repo);
  }
  
  private async getRepositoryDetails(repository: any): Promise<any> {
    switch (repository.type) {
      case 'local': {
        const localRepo = repository as LocalRepository;
        const stats = await fs.stat(localRepo.path);
        return {
          path: localRepo.path,
          size: stats.size,
          modified: stats.mtime,
          // Could add git info like current branch, commit hash, etc.
        };
      }
      case 'github':
      case 'gitlab': {
        const remoteRepo = repository as RemoteRepository;
        return {
          url: remoteRepo.url,
          branch: remoteRepo.branch,
          commit: remoteRepo.commit,
          // Could add API info like last commit date, repository stats, etc.
        };
      }
      default:
        return {};
    }
  }
}

export function createEnvironmentService(app: FastifyInstance, storage: StorageBackend): EnvironmentService {
  return new EnvironmentService(storage);
}
```

### 2. Repository Utilities

#### server/utils/repository.ts
```typescript
import { exec } from 'child_process';
import { promisify } from 'util';
import type { RemoteRepository } from '@shared/types';
import { ValidationError } from '@shared/constants/errors.js';

const execAsync = promisify(exec);

export async function validateRepositoryAccess(repo: RemoteRepository): Promise<void> {
  try {
    // For GitHub/GitLab, we can use git ls-remote to check accessibility
    const command = `git ls-remote --heads "${repo.url}"`;
    
    const env = { ...process.env };
    if (repo.accessToken) {
      // Add token to URL for authentication
      const url = new URL(repo.url);
      url.username = 'token';
      url.password = repo.accessToken;
      const authenticatedCommand = `git ls-remote --heads "${url.toString()}"`;
      await execAsync(authenticatedCommand, { env, timeout: 10000 });
    } else {
      await execAsync(command, { env, timeout: 10000 });
    }
  } catch (error) {
    throw new ValidationError(
      `Cannot access remote repository: ${error instanceof Error ? error.message : String(error)}`,
      { url: repo.url }
    );
  }
}

export async function getRemoteRepositoryInfo(repo: RemoteRepository): Promise<{
  branches: string[];
  defaultBranch: string;
  lastCommit?: string;
}> {
  try {
    const command = `git ls-remote --heads --symref "${repo.url}"`;
    const { stdout } = await execAsync(command, { timeout: 10000 });
    
    const lines = stdout.trim().split('\n');
    const branches: string[] = [];
    let defaultBranch = 'main';
    
    for (const line of lines) {
      if (line.startsWith('ref: refs/heads/')) {
        // This is the default branch reference
        defaultBranch = line.split('/').pop() || 'main';
      } else if (line.includes('refs/heads/')) {
        // This is a branch
        const branchName = line.split('refs/heads/')[1];
        if (branchName) {
          branches.push(branchName);
        }
      }
    }
    
    return {
      branches,
      defaultBranch,
    };
  } catch (error) {
    throw new ValidationError(
      `Cannot get repository info: ${error instanceof Error ? error.message : String(error)}`,
      { url: repo.url }
    );
  }
}

export function parseGitHubUrl(url: string): {
  owner: string;
  repo: string;
  isValid: boolean;
} {
  try {
    const parsed = new URL(url);
    if (parsed.hostname !== 'github.com') {
      return { owner: '', repo: '', isValid: false };
    }
    
    const pathParts = parsed.pathname.split('/').filter(p => p);
    if (pathParts.length < 2) {
      return { owner: '', repo: '', isValid: false };
    }
    
    return {
      owner: pathParts[0],
      repo: pathParts[1].replace(/\.git$/, ''),
      isValid: true
    };
  } catch {
    return { owner: '', repo: '', isValid: false };
  }
}

export function parseGitLabUrl(url: string): {
  namespace: string;
  project: string;
  isValid: boolean;
} {
  try {
    const parsed = new URL(url);
    if (!parsed.hostname.includes('gitlab')) {
      return { namespace: '', project: '', isValid: false };
    }
    
    const pathParts = parsed.pathname.split('/').filter(p => p);
    if (pathParts.length < 2) {
      return { namespace: '', project: '', isValid: false };
    }
    
    return {
      namespace: pathParts[0],
      project: pathParts[1].replace(/\.git$/, ''),
      isValid: true
    };
  } catch {
    return { namespace: '', project: '', isValid: false };
  }
}
```

### 3. File Search Utilities

#### server/utils/file-search.ts
Port from: `swarm/utils/file_search.py`

```typescript
import { promises as fs } from 'fs';
import path from 'path';
import { glob } from 'glob';
import type { FileSearchResponse } from '@shared/types';

export async function searchFilesInDirectory(
  directory: string,
  query: string,
  limit: number = 100,
  filePattern?: string
): Promise<FileSearchResponse> {
  try {
    // Build glob pattern
    let pattern: string;
    if (filePattern) {
      pattern = `**/${filePattern}`;
    } else {
      pattern = `**/*${query}*`;
    }
    
    const files = await glob(pattern, {
      cwd: directory,
      nodir: true,
      ignore: [
        '**/node_modules/**',
        '**/.git/**',
        '**/dist/**',
        '**/build/**',
        '**/.next/**',
        '**/.vscode/**',
        '**/.idea/**',
        '**/coverage/**',
        '**/*.log',
        '**/.DS_Store',
        '**/Thumbs.db'
      ],
      absolute: false,
      maxDepth: 20,
      follow: false // Don't follow symlinks
    });
    
    // Filter by query if filePattern was provided
    let filteredFiles = files;
    if (filePattern && query) {
      filteredFiles = files.filter(file => 
        file.toLowerCase().includes(query.toLowerCase()) ||
        path.basename(file).toLowerCase().includes(query.toLowerCase())
      );
    }
    
    // Sort by relevance (exact matches first, then alphabetical)
    filteredFiles.sort((a, b) => {
      const aBasename = path.basename(a).toLowerCase();
      const bBasename = path.basename(b).toLowerCase();
      const queryLower = query.toLowerCase();
      
      // Exact matches first
      const aExact = aBasename === queryLower ? 0 : 1;
      const bExact = bBasename === queryLower ? 0 : 1;
      if (aExact !== bExact) return aExact - bExact;
      
      // Starts with query
      const aStarts = aBasename.startsWith(queryLower) ? 0 : 1;
      const bStarts = bBasename.startsWith(queryLower) ? 0 : 1;
      if (aStarts !== bStarts) return aStarts - bStarts;
      
      // Alphabetical
      return a.localeCompare(b);
    });
    
    // Apply limit
    const limitedFiles = filteredFiles.slice(0, limit);
    
    return {
      files: limitedFiles,
      totalCount: filteredFiles.length,
      truncated: filteredFiles.length > limit
    };
  } catch (error) {
    throw new Error(`File search failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function searchFileContents(
  directory: string,
  query: string,
  filePattern?: string,
  limit: number = 50
): Promise<{
  files: Array<{
    path: string;
    matches: Array<{
      line: number;
      content: string;
      column: number;
    }>;
  }>;
  totalMatches: number;
}> {
  try {
    // Get list of files to search
    const pattern = filePattern || '**/*';
    const files = await glob(pattern, {
      cwd: directory,
      nodir: true,
      ignore: [
        '**/node_modules/**',
        '**/.git/**',
        '**/dist/**',
        '**/build/**',
        '**/*.min.js',
        '**/*.map',
        '**/*.log',
        '**/coverage/**',
        '**/*.png',
        '**/*.jpg',
        '**/*.gif',
        '**/*.pdf',
        '**/*.zip'
      ],
      absolute: false,
      maxDepth: 10
    });
    
    const results: Array<{
      path: string;
      matches: Array<{
        line: number;
        content: string;
        column: number;
      }>;
    }> = [];
    
    let totalMatches = 0;
    const regex = new RegExp(query, 'gi');
    
    for (const file of files.slice(0, limit * 2)) { // Search more files than limit
      try {
        const fullPath = path.join(directory, file);
        const content = await fs.readFile(fullPath, 'utf-8');
        const lines = content.split('\n');
        
        const matches: Array<{
          line: number;
          content: string;
          column: number;
        }> = [];
        
        for (let i = 0; i < lines.length; i++) {
          const line = lines[i];
          let match;
          regex.lastIndex = 0; // Reset regex state
          
          while ((match = regex.exec(line)) !== null) {
            matches.push({
              line: i + 1,
              content: line.trim(),
              column: match.index + 1
            });
            totalMatches++;
          }
        }
        
        if (matches.length > 0) {
          results.push({
            path: file,
            matches: matches.slice(0, 10) // Limit matches per file
          });
        }
        
        if (results.length >= limit) break;
      } catch (error) {
        // Skip files that can't be read (binary files, permission issues, etc.)
        continue;
      }
    }
    
    return {
      files: results,
      totalMatches
    };
  } catch (error) {
    throw new Error(`Content search failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}

export async function getFileStats(directory: string): Promise<{
  totalFiles: number;
  totalSize: number;
  fileTypes: Record<string, number>;
}> {
  try {
    const files = await glob('**/*', {
      cwd: directory,
      nodir: true,
      ignore: ['**/node_modules/**', '**/.git/**'],
      absolute: true,
      maxDepth: 10
    });
    
    let totalSize = 0;
    const fileTypes: Record<string, number> = {};
    
    for (const file of files) {
      try {
        const stats = await fs.stat(file);
        totalSize += stats.size;
        
        const ext = path.extname(file).toLowerCase() || 'no-extension';
        fileTypes[ext] = (fileTypes[ext] || 0) + 1;
      } catch {
        // Skip files that can't be accessed
        continue;
      }
    }
    
    return {
      totalFiles: files.length,
      totalSize,
      fileTypes
    };
  } catch (error) {
    throw new Error(`File stats failed: ${error instanceof Error ? error.message : String(error)}`);
  }
}
```

### 4. Environment Templates

#### server/templates/environment-templates.ts
```typescript
import type { CreateEnvironmentRequest } from '@shared/types';

export const environmentTemplates: Record<string, Partial<CreateEnvironmentRequest>> = {
  'local-nodejs': {
    name: 'Local Node.js Project',
    description: 'Local Node.js/TypeScript project environment',
    codeExecution: {
      type: 'local',
      config: {
        nodeVersion: '18',
        packageManager: 'npm'
      }
    }
  },
  
  'local-python': {
    name: 'Local Python Project',
    description: 'Local Python project environment',
    codeExecution: {
      type: 'local',
      config: {
        pythonVersion: '3.9',
        packageManager: 'pip'
      }
    }
  },
  
  'github-repo': {
    name: 'GitHub Repository',
    description: 'Remote GitHub repository environment',
    codeExecution: {
      type: 'docker',
      config: {
        image: 'node:18-alpine'
      }
    }
  },
  
  'docker-sandbox': {
    name: 'Docker Sandbox',
    description: 'Isolated Docker container environment',
    codeExecution: {
      type: 'docker',
      config: {
        image: 'ubuntu:22.04',
        workdir: '/workspace'
      }
    }
  }
};

export function getEnvironmentTemplate(templateName: string): Partial<CreateEnvironmentRequest> | null {
  return environmentTemplates[templateName] || null;
}

export function listEnvironmentTemplates(): Array<{
  name: string;
  template: Partial<CreateEnvironmentRequest>;
}> {
  return Object.entries(environmentTemplates).map(([name, template]) => ({
    name,
    template
  }));
}
```

## Testing Strategy

### Unit Tests

#### server/services/__tests__/environment.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { EnvironmentService } from '../environment.js';
import { FileStorage } from '../storage.js';
import { promises as fs } from 'fs';
import path from 'path';

describe('EnvironmentService', () => {
  let service: EnvironmentService;
  let storage: FileStorage;
  let tempDir: string;
  
  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now());
    await fs.mkdir(tempDir, { recursive: true });
    storage = new FileStorage(tempDir);
    service = new EnvironmentService(storage);
  });
  
  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });
  
  describe('createEnvironment', () => {
    it('should create a local environment', async () => {
      // Create a test git repository
      const repoPath = path.join(tempDir, 'test-repo');
      await fs.mkdir(repoPath, { recursive: true });
      await fs.mkdir(path.join(repoPath, '.git'), { recursive: true });
      
      const request = {
        name: 'Test Environment',
        repository: {
          type: 'local' as const,
          path: repoPath
        },
        codeExecution: {
          type: 'local' as const
        }
      };
      
      const environment = await service.createEnvironment(request);
      
      expect(environment.id).toBeDefined();
      expect(environment.name).toBe('Test Environment');
      expect(environment.repository.type).toBe('local');
    });
    
    it('should validate repository access', async () => {
      const request = {
        name: 'Invalid Environment',
        repository: {
          type: 'local' as const,
          path: '/non/existent/path'
        },
        codeExecution: {
          type: 'local' as const
        }
      };
      
      await expect(service.createEnvironment(request))
        .rejects.toThrow('Cannot access repository');
    });
  });
  
  describe('searchFiles', () => {
    it('should search files in local repository', async () => {
      // Create test environment and files
      const repoPath = path.join(tempDir, 'test-repo');
      await fs.mkdir(repoPath, { recursive: true });
      await fs.mkdir(path.join(repoPath, '.git'), { recursive: true });
      await fs.writeFile(path.join(repoPath, 'test.js'), 'console.log("test");');
      await fs.writeFile(path.join(repoPath, 'README.md'), '# Test Project');
      
      const environment = await service.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: repoPath
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const results = await service.searchFiles({
        environmentId: environment.id,
        query: 'test'
      });
      
      expect(results.files).toContain('test.js');
      expect(results.totalCount).toBeGreaterThan(0);
    });
  });
});
```

## Migration Checklist

- [ ] Create environment service with CRUD operations
- [ ] Implement repository validation for local and remote repos
- [ ] Add file search functionality
- [ ] Create repository utility functions
- [ ] Add environment templates
- [ ] Implement git operations for remote repositories
- [ ] Add environment validation and health checks
- [ ] Create comprehensive error handling
- [ ] Add file content search capabilities
- [ ] Write unit tests for all functionality
- [ ] Add integration tests with real repositories
- [ ] Validate performance with large repositories
- [ ] Test error scenarios and edge cases

## Best Practices

### 1. Repository Security
- Validate all repository paths and URLs
- Sanitize user inputs for file operations
- Use read-only access when possible
- Implement timeout for git operations

### 2. Performance
- Use efficient file globbing patterns
- Implement caching for frequently accessed data
- Limit search results to prevent memory issues
- Use streaming for large file operations

### 3. Error Handling
- Provide meaningful error messages
- Handle repository access failures gracefully
- Validate repository state before operations
- Log errors with sufficient context

## Next Steps

1. Review [Workspace Management Migration](./06-workspace-management.md)
2. Implement GitHub/GitLab API integration
3. Add advanced file search features
4. Set up repository caching strategies
5. Configure security scanning for repositories