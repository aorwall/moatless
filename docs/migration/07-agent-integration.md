# Agent Integration Migration Guide

This guide covers the migration of agent integration functionality from Python to TypeScript.

## Overview

Phase 7 focuses on migrating the agent system that handles Claude Code CLI integration, process management, and agent lifecycle management for executing AI-powered tasks.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/agents/base.py` - Base agent class
- `/moatless-swarm/swarm/agents/claude_code.py` - Claude Code agent implementation
- `/moatless-swarm/swarm/agents/options.py` - Agent options/configuration
- `/moatless-swarm/swarm/utils/subprocess.py` - Subprocess utilities for CLI integration

## Implementation

### 1. Base Agent Interface

#### server/services/agents/base-agent.ts
Port from: `swarm/agents/base.py`

```typescript
import type { Event } from '@shared/types/event';

export interface AgentOptions {
  model?: string;
  apiKey?: string;
  maxTokens?: number;
  temperature?: number;
  timeout?: number;
  [key: string]: any;
}

export interface AgentRunOptions {
  workspaceDir?: string;
  environmentVariables?: Record<string, string>;
  timeout?: number;
  onProgress?: (event: Event) => void;
}

export abstract class BaseAgent {
  protected stopRequested = false;
  
  constructor(
    protected taskId: string,
    protected options: AgentOptions = {}
  ) {}
  
  abstract run(message: string, options?: AgentRunOptions): AsyncGenerator<Event>;
  abstract stop(): Promise<void>;
  
  protected createEvent(
    type: string,
    payload: any,
    isError: boolean = false,
    subType?: string
  ): Event {
    return {
      eventId: crypto.randomUUID(),
      type: type as any,
      subType,
      taskId: this.taskId,
      timestamp: new Date(),
      isError,
      payload
    };
  }
  
  protected async delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}
```

### 2. Claude Code Agent Implementation

#### server/services/agents/claude-code-agent.ts
Port from: `swarm/agents/claude_code.py`

```typescript
import { spawn, ChildProcess } from 'child_process';
import { createInterface, Interface } from 'readline';
import { promises as fs } from 'fs';
import path from 'path';
import type { Event } from '@shared/types/event';
import { BaseAgent, AgentOptions, AgentRunOptions } from './base-agent.js';

export interface ClaudeCodeOptions extends AgentOptions {
  cliPath?: string;
  model?: string;
  apiKey?: string;
  maxTokens?: number;
  temperature?: number;
  systemPrompt?: string;
  tools?: string[];
  outputFormat?: 'json' | 'text';
}

export class ClaudeCodeAgent extends BaseAgent {
  private process?: ChildProcess;
  private readline?: Interface;
  private cliPath: string;
  
  constructor(
    taskId: string,
    options: ClaudeCodeOptions = {},
    cliPath?: string
  ) {
    super(taskId, options);
    this.cliPath = cliPath || this.findCli();
  }
  
  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false;
    
    try {
      yield this.createEvent('start', {
        message,
        agentType: 'claude-code',
        options: this.options
      });
      
      const args = this.buildCliArgs(message);
      
      // Spawn claude process
      this.process = spawn(this.cliPath, args, {
        cwd: runOptions.workspaceDir || process.cwd(),
        env: this.getEnvironment(runOptions.environmentVariables),
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: false
      });
      
      // Handle process errors
      this.process.on('error', (error) => {
        console.error('Process error:', error);
      });
      
      // Create readline interface for stdout
      this.readline = createInterface({
        input: this.process.stdout!,
        crlfDelay: Infinity
      });
      
      // Handle stderr
      let stderrOutput = '';
      this.process.stderr?.on('data', (data) => {
        stderrOutput += data.toString();
      });
      
      // Process each line of output
      let lastEvent: Event | null = null;
      
      for await (const line of this.readline) {
        if (this.stopRequested) {
          yield this.createEvent('stop', { reason: 'user_requested' });
          break;
        }
        
        try {
          // Try to parse as JSON (Claude Code outputs structured events)
          const data = JSON.parse(line);
          const event = this.parseEventFromJson(data);
          lastEvent = event;
          yield event;
        } catch (e) {
          // Non-JSON output - treat as message content
          if (line.trim()) {
            const event = this.createEvent('message', {
              content: line,
              role: 'assistant'
            });
            lastEvent = event;
            yield event;
          }
        }
      }
      
      // Wait for process to exit
      const exitCode = await this.waitForProcessExit();
      
      if (exitCode === 0) {
        yield this.createEvent('completion', {
          exitCode,
          successful: true,
          lastEvent: lastEvent?.payload
        });
      } else {
        yield this.createEvent('error', {
          exitCode,
          error: `Process exited with code ${exitCode}`,
          stderr: stderrOutput
        }, true);
      }
      
    } catch (error) {
      yield this.createEvent('error', {
        error: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined
      }, true);
    } finally {
      await this.cleanup();
    }
  }
  
  async stop(): Promise<void> {
    this.stopRequested = true;
    
    if (this.process && !this.process.killed) {
      // Send SIGTERM first
      this.process.kill('SIGTERM');
      
      // Give it 5 seconds to gracefully shutdown
      const timeout = new Promise<void>((resolve) => {
        setTimeout(() => {
          if (this.process && !this.process.killed) {
            this.process.kill('SIGKILL');
          }
          resolve();
        }, 5000);
      });
      
      const processExit = new Promise<void>((resolve) => {
        if (this.process) {
          this.process.on('close', () => resolve());
        } else {
          resolve();
        }
      });
      
      await Promise.race([timeout, processExit]);
    }
    
    await this.cleanup();
  }
  
  private findCli(): string {
    // Port logic from _find_cli method
    const possiblePaths = [
      process.env.CLAUDE_CLI_PATH,
      path.join(process.env.HOME || '', '.npm-global/bin/claude'),
      path.join(process.env.HOME || '', '.local/bin/claude'),
      '/usr/local/bin/claude',
      '/opt/homebrew/bin/claude'
    ].filter(Boolean) as string[];
    
    // Check which path exists
    for (const cliPath of possiblePaths) {
      try {
        require('fs').accessSync(cliPath, require('fs').constants.F_OK);
        return cliPath;
      } catch {
        continue;
      }
    }
    
    // Default to PATH lookup
    return 'claude';
  }
  
  private buildCliArgs(message: string): string[] {
    const args: string[] = [];
    const options = this.options as ClaudeCodeOptions;
    
    // Add model if specified
    if (options.model) {
      args.push('--model', options.model);
    }
    
    // Add API key if specified
    if (options.apiKey) {
      args.push('--api-key', options.apiKey);
    }
    
    // Add max tokens
    if (options.maxTokens) {
      args.push('--max-tokens', String(options.maxTokens));
    }
    
    // Add temperature
    if (options.temperature !== undefined) {
      args.push('--temperature', String(options.temperature));
    }
    
    // Add system prompt
    if (options.systemPrompt) {
      args.push('--system', options.systemPrompt);
    }
    
    // Add tools
    if (options.tools && options.tools.length > 0) {
      for (const tool of options.tools) {
        args.push('--tool', tool);
      }
    }
    
    // Add output format
    if (options.outputFormat) {
      args.push('--output', options.outputFormat);
    }
    
    // Add the message
    args.push(message);
    
    return args;
  }
  
  private getEnvironment(additionalEnv?: Record<string, string>): NodeJS.ProcessEnv {
    const options = this.options as ClaudeCodeOptions;
    
    return {
      ...process.env,
      ...additionalEnv,
      CLAUDE_API_KEY: options.apiKey || process.env.CLAUDE_API_KEY,
      NO_COLOR: '1', // Disable color output for easier parsing
      FORCE_COLOR: '0',
      TERM: 'dumb' // Ensure no terminal-specific behavior
    };
  }
  
  private parseEventFromJson(data: any): Event {
    // Transform claude CLI output to our event format
    const eventType = this.mapClaudeEventType(data.type || 'message');
    
    return this.createEvent(
      eventType,
      {
        ...data,
        originalType: data.type,
        content: data.content || data.message || data.text,
        tool: data.tool,
        toolInput: data.tool_input,
        toolOutput: data.tool_output
      },
      data.error || false,
      data.subtype || data.sub_type
    );
  }
  
  private mapClaudeEventType(claudeType: string): string {
    // Map Claude CLI event types to our internal event types
    const mapping: Record<string, string> = {
      'text': 'message',
      'tool_use': 'tool',
      'tool_result': 'tool',
      'error': 'error',
      'completion': 'completion',
      'input': 'input',
      'thinking': 'message'
    };
    
    return mapping[claudeType] || 'message';
  }
  
  private async waitForProcessExit(): Promise<number> {
    return new Promise((resolve, reject) => {
      if (!this.process) {
        resolve(0);
        return;
      }
      
      this.process.on('close', (code) => {
        resolve(code || 0);
      });
      
      this.process.on('error', (error) => {
        reject(error);
      });
      
      // Timeout after configured time or 5 minutes
      const timeout = (this.options.timeout || 300) * 1000;
      setTimeout(() => {
        if (this.process && !this.process.killed) {
          this.process.kill('SIGKILL');
          reject(new Error('Process timeout'));
        }
      }, timeout);
    });
  }
  
  private async cleanup(): Promise<void> {
    if (this.readline) {
      this.readline.close();
      this.readline = undefined;
    }
    
    if (this.process && !this.process.killed) {
      this.process.kill('SIGKILL');
    }
    
    this.process = undefined;
  }
}
```

### 3. Agent Options Configuration

#### server/services/agents/agent-options.ts
Port from: `swarm/agents/options.py`

```typescript
export interface BaseAgentOptions {
  model?: string;
  apiKey?: string;
  maxTokens?: number;
  temperature?: number;
  timeout?: number;
  systemPrompt?: string;
}

export interface ClaudeCodeOptions extends BaseAgentOptions {
  cliPath?: string;
  tools?: string[];
  outputFormat?: 'json' | 'text';
  workspaceIntegration?: boolean;
  autoApprove?: boolean;
  maxIterations?: number;
}

export interface OpenAIAgentOptions extends BaseAgentOptions {
  organization?: string;
  functions?: any[];
  functionCall?: 'auto' | 'none' | { name: string };
}

export class AgentOptionsBuilder {
  private options: Record<string, any> = {};
  
  model(model: string): this {
    this.options.model = model;
    return this;
  }
  
  apiKey(apiKey: string): this {
    this.options.apiKey = apiKey;
    return this;
  }
  
  maxTokens(maxTokens: number): this {
    this.options.maxTokens = maxTokens;
    return this;
  }
  
  temperature(temperature: number): this {
    this.options.temperature = temperature;
    return this;
  }
  
  timeout(timeout: number): this {
    this.options.timeout = timeout;
    return this;
  }
  
  systemPrompt(prompt: string): this {
    this.options.systemPrompt = prompt;
    return this;
  }
  
  tools(tools: string[]): this {
    this.options.tools = tools;
    return this;
  }
  
  outputFormat(format: 'json' | 'text'): this {
    this.options.outputFormat = format;
    return this;
  }
  
  build<T = BaseAgentOptions>(): T {
    return { ...this.options } as T;
  }
}

export const defaultClaudeCodeOptions: ClaudeCodeOptions = {
  model: 'claude-3-5-sonnet-20241022',
  maxTokens: 8192,
  temperature: 0.1,
  timeout: 300, // 5 minutes
  tools: ['computer', 'bash', 'str_replace_editor'],
  outputFormat: 'json',
  workspaceIntegration: true,
  autoApprove: false,
  maxIterations: 10
};

export function createClaudeCodeOptions(overrides: Partial<ClaudeCodeOptions> = {}): ClaudeCodeOptions {
  return {
    ...defaultClaudeCodeOptions,
    ...overrides
  };
}

export function validateAgentOptions(options: BaseAgentOptions): string[] {
  const errors: string[] = [];
  
  if (options.maxTokens && (options.maxTokens < 1 || options.maxTokens > 200000)) {
    errors.push('maxTokens must be between 1 and 200000');
  }
  
  if (options.temperature !== undefined && (options.temperature < 0 || options.temperature > 2)) {
    errors.push('temperature must be between 0 and 2');
  }
  
  if (options.timeout && options.timeout < 1) {
    errors.push('timeout must be at least 1 second');
  }
  
  return errors;
}
```

### 4. Agent Service

#### server/services/agent.ts
```typescript
import { randomUUID } from 'crypto';
import type { FastifyInstance } from 'fastify';
import type { BaseAgent } from './agents/base-agent.js';
import { ClaudeCodeAgent, ClaudeCodeOptions } from './agents/claude-code-agent.js';
import type { TaskService } from './task.js';
import type { WorkspaceService } from './workspace.js';
import type { EventService } from './event.js';
import { createClaudeCodeOptions, validateAgentOptions } from './agents/agent-options.js';
import { ValidationError } from '@shared/constants/errors.js';

export type AgentType = 'claude-code' | 'openai' | 'custom';

export interface AgentExecution {
  id: string;
  taskId: string;
  agentType: AgentType;
  status: 'running' | 'completed' | 'error' | 'stopped';
  startedAt: Date;
  completedAt?: Date;
  error?: string;
  agent?: BaseAgent;
}

export class AgentService {
  private activeAgents: Map<string, AgentExecution> = new Map();
  private agentRegistry: Map<AgentType, typeof BaseAgent> = new Map();
  
  constructor(
    private taskService: TaskService,
    private workspaceService: WorkspaceService,
    private eventService: EventService
  ) {
    this.registerDefaultAgents();
  }
  
  async runAgent(
    taskId: string, 
    message: string, 
    agentType: AgentType = 'claude-code',
    options: any = {}
  ): Promise<string> {
    // Check if agent is already running for this task
    const existingExecution = Array.from(this.activeAgents.values())
      .find(exec => exec.taskId === taskId && exec.status === 'running');
    
    if (existingExecution) {
      throw new ValidationError('Agent is already running for this task', { taskId, executionId: existingExecution.id });
    }
    
    // Get task metadata
    const task = await this.taskService.getTask(taskId);
    if (!task) {
      throw new ValidationError(`Task ${taskId} not found`, { taskId });
    }
    
    // Get workspace
    const workspace = await this.workspaceService.getWorkspace(taskId);
    if (!workspace) {
      throw new ValidationError(`Workspace for task ${taskId} not found`, { taskId });
    }
    
    // Validate options
    const validationErrors = validateAgentOptions(options);
    if (validationErrors.length > 0) {
      throw new ValidationError('Invalid agent options', { errors: validationErrors });
    }
    
    // Create agent execution record
    const executionId = randomUUID();
    const execution: AgentExecution = {
      id: executionId,
      taskId,
      agentType,
      status: 'running',
      startedAt: new Date()
    };
    
    this.activeAgents.set(executionId, execution);
    
    // Create and run agent asynchronously
    this.runAgentAsync(execution, workspace, message, options)
      .catch(error => {
        console.error(`Agent execution ${executionId} failed:`, error);
      });
    
    return executionId;
  }
  
  async stopAgent(executionId: string): Promise<void> {
    const execution = this.activeAgents.get(executionId);
    if (!execution) {
      throw new ValidationError(`Agent execution ${executionId} not found`, { executionId });
    }
    
    if (execution.status !== 'running') {
      throw new ValidationError(`Agent execution ${executionId} is not running`, { 
        executionId, 
        status: execution.status 
      });
    }
    
    if (execution.agent) {
      await execution.agent.stop();
    }
    
    execution.status = 'stopped';
    execution.completedAt = new Date();
    
    // Emit stop event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'stop',
      taskId: execution.taskId,
      timestamp: new Date(),
      isError: false,
      payload: { 
        executionId,
        reason: 'user_requested'
      }
    });
  }
  
  async getAgentExecution(executionId: string): Promise<AgentExecution | null> {
    return this.activeAgents.get(executionId) || null;
  }
  
  async listActiveAgents(): Promise<AgentExecution[]> {
    return Array.from(this.activeAgents.values())
      .filter(exec => exec.status === 'running');
  }
  
  async listAgentExecutions(taskId?: string): Promise<AgentExecution[]> {
    const executions = Array.from(this.activeAgents.values());
    
    if (taskId) {
      return executions.filter(exec => exec.taskId === taskId);
    }
    
    return executions;
  }
  
  registerAgent(type: AgentType, agentClass: typeof BaseAgent): void {
    this.agentRegistry.set(type, agentClass);
  }
  
  private registerDefaultAgents(): void {
    this.agentRegistry.set('claude-code', ClaudeCodeAgent as any);
    // Additional agents can be registered here
  }
  
  private async runAgentAsync(
    execution: AgentExecution,
    workspace: any,
    message: string,
    options: any
  ): Promise<void> {
    try {
      // Create agent based on type
      const agent = this.createAgent(execution.agentType, execution.taskId, options);
      execution.agent = agent;
      
      // Prepare run options
      const runOptions = {
        workspaceDir: workspace.type === 'local' ? workspace.path : undefined,
        timeout: options.timeout,
        onProgress: async (event: any) => {
          await this.eventService.emit(event);
        }
      };
      
      // Run agent and stream events
      for await (const event of agent.run(message, runOptions)) {
        if (execution.status !== 'running') {
          break;
        }
        
        await this.eventService.emit(event);
      }
      
      execution.status = 'completed';
      execution.completedAt = new Date();
      
    } catch (error) {
      execution.status = 'error';
      execution.completedAt = new Date();
      execution.error = error instanceof Error ? error.message : String(error);
      
      // Emit error event
      await this.eventService.emit({
        eventId: randomUUID(),
        type: 'error',
        taskId: execution.taskId,
        timestamp: new Date(),
        isError: true,
        payload: { 
          executionId: execution.id,
          error: execution.error 
        }
      });
    } finally {
      // Clean up agent reference
      if (execution.agent) {
        execution.agent = undefined;
      }
    }
  }
  
  private createAgent(agentType: AgentType, taskId: string, options: any): BaseAgent {
    const AgentClass = this.agentRegistry.get(agentType);
    if (!AgentClass) {
      throw new ValidationError(`Unknown agent type: ${agentType}`, { agentType });
    }
    
    switch (agentType) {
      case 'claude-code':
        const claudeOptions = createClaudeCodeOptions(options);
        return new ClaudeCodeAgent(taskId, claudeOptions) as BaseAgent;
      default:
        return new AgentClass(taskId, options);
    }
  }
}

export function createAgentService(
  app: FastifyInstance,
  taskService: TaskService,
  workspaceService: WorkspaceService,
  eventService: EventService
): AgentService {
  return new AgentService(taskService, workspaceService, eventService);
}
```

### 5. Process Management Utilities

#### server/utils/process-utils.ts
Port from: `swarm/utils/subprocess.py`

```typescript
import { spawn, exec, ChildProcess } from 'child_process';
import { promisify } from 'util';
import { EventEmitter } from 'events';

const execAsync = promisify(exec);

export interface ProcessOptions {
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  timeout?: number;
  shell?: boolean;
  stdio?: 'pipe' | 'inherit' | 'ignore';
  maxBuffer?: number;
}

export interface ProcessResult {
  stdout: string;
  stderr: string;
  exitCode: number;
  signal?: string;
  timedOut: boolean;
}

export class ProcessManager extends EventEmitter {
  private activeProcesses: Map<string, ChildProcess> = new Map();
  
  async execute(
    command: string,
    args: string[] = [],
    options: ProcessOptions = {}
  ): Promise<ProcessResult> {
    const processId = crypto.randomUUID();
    
    return new Promise((resolve, reject) => {
      const child = spawn(command, args, {
        cwd: options.cwd,
        env: options.env || process.env,
        stdio: options.stdio || 'pipe',
        shell: options.shell || false
      });
      
      this.activeProcesses.set(processId, child);
      
      let stdout = '';
      let stderr = '';
      let timedOut = false;
      
      // Collect output
      if (child.stdout) {
        child.stdout.on('data', (data) => {
          stdout += data.toString();
          this.emit('stdout', processId, data.toString());
        });
      }
      
      if (child.stderr) {
        child.stderr.on('data', (data) => {
          stderr += data.toString();
          this.emit('stderr', processId, data.toString());
        });
      }
      
      // Handle process completion
      child.on('close', (code, signal) => {
        this.activeProcesses.delete(processId);
        
        resolve({
          stdout,
          stderr,
          exitCode: code || 0,
          signal: signal || undefined,
          timedOut
        });
      });
      
      child.on('error', (error) => {
        this.activeProcesses.delete(processId);
        reject(error);
      });
      
      // Handle timeout
      if (options.timeout) {
        setTimeout(() => {
          if (this.activeProcesses.has(processId)) {
            timedOut = true;
            child.kill('SIGTERM');
            
            // Force kill after 5 seconds
            setTimeout(() => {
              if (this.activeProcesses.has(processId)) {
                child.kill('SIGKILL');
              }
            }, 5000);
          }
        }, options.timeout);
      }
    });
  }
  
  async executeShell(
    command: string,
    options: ProcessOptions = {}
  ): Promise<ProcessResult> {
    const processId = crypto.randomUUID();
    
    try {
      const result = await execAsync(command, {
        cwd: options.cwd,
        env: options.env || process.env,
        timeout: options.timeout,
        maxBuffer: options.maxBuffer || 1024 * 1024 // 1MB default
      });
      
      return {
        stdout: result.stdout,
        stderr: result.stderr,
        exitCode: 0,
        timedOut: false
      };
    } catch (error: any) {
      return {
        stdout: error.stdout || '',
        stderr: error.stderr || '',
        exitCode: error.code || 1,
        signal: error.signal,
        timedOut: error.killed && error.signal === 'SIGTERM'
      };
    }
  }
  
  killProcess(processId: string, signal: NodeJS.Signals = 'SIGTERM'): boolean {
    const process = this.activeProcesses.get(processId);
    if (process && !process.killed) {
      process.kill(signal);
      return true;
    }
    return false;
  }
  
  killAllProcesses(signal: NodeJS.Signals = 'SIGTERM'): void {
    for (const [processId, process] of this.activeProcesses) {
      if (!process.killed) {
        process.kill(signal);
      }
    }
  }
  
  getActiveProcessCount(): number {
    return this.activeProcesses.size;
  }
  
  async cleanup(): Promise<void> {
    this.killAllProcesses('SIGTERM');
    
    // Wait for processes to exit gracefully
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    // Force kill any remaining processes
    this.killAllProcesses('SIGKILL');
    
    this.activeProcesses.clear();
  }
}

export const processManager = new ProcessManager();

// Cleanup on process exit
process.on('exit', () => {
  processManager.cleanup();
});

process.on('SIGINT', () => {
  processManager.cleanup().then(() => process.exit(0));
});

process.on('SIGTERM', () => {
  processManager.cleanup().then(() => process.exit(0));
});
```

## Testing Strategy

### Unit Tests

#### server/services/__tests__/agent.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { AgentService } from '../agent.js';
import { FileStorage } from '../storage.js';
import { EventService } from '../event.js';
import { TaskService } from '../task.js';
import { WorkspaceService } from '../workspace.js';
import { EnvironmentService } from '../environment.js';
import { promises as fs } from 'fs';
import path from 'path';

// Mock the Claude CLI
vi.mock('child_process', () => ({
  spawn: vi.fn(),
  exec: vi.fn()
}));

describe('AgentService', () => {
  let service: AgentService;
  let storage: FileStorage;
  let eventService: EventService;
  let taskService: TaskService;
  let workspaceService: WorkspaceService;
  let environmentService: EnvironmentService;
  let tempDir: string;
  
  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now());
    await fs.mkdir(tempDir, { recursive: true });
    
    storage = new FileStorage(tempDir);
    eventService = new EventService(storage);
    environmentService = new EnvironmentService(storage);
    workspaceService = new WorkspaceService(storage, environmentService, eventService);
    taskService = new TaskService(storage, eventService, environmentService, workspaceService);
    service = new AgentService(taskService, workspaceService, eventService);
  });
  
  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });
  
  describe('runAgent', () => {
    it('should start agent execution', async () => {
      // Create test environment and workspace
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false
      });
      
      await workspaceService.createWorkspace(taskId, environment.id);
      
      const executionId = await service.runAgent(taskId, 'Hello world');
      
      expect(executionId).toBeDefined();
      
      const execution = await service.getAgentExecution(executionId);
      expect(execution).toBeDefined();
      expect(execution?.taskId).toBe(taskId);
      expect(execution?.status).toBe('running');
    });
    
    it('should prevent multiple agents for same task', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false
      });
      
      await workspaceService.createWorkspace(taskId, environment.id);
      
      // Start first agent
      await service.runAgent(taskId, 'First message');
      
      // Try to start second agent - should fail
      await expect(service.runAgent(taskId, 'Second message'))
        .rejects.toThrow('Agent is already running');
    });
  });
  
  describe('stopAgent', () => {
    it('should stop running agent', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false
      });
      
      await workspaceService.createWorkspace(taskId, environment.id);
      
      const executionId = await service.runAgent(taskId, 'Hello world');
      
      // Stop the agent
      await service.stopAgent(executionId);
      
      const execution = await service.getAgentExecution(executionId);
      expect(execution?.status).toBe('stopped');
      expect(execution?.completedAt).toBeDefined();
    });
  });
});
```

### Integration Tests

#### server/services/__tests__/claude-code-integration.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { ClaudeCodeAgent } from '../agents/claude-code-agent.js';
import { promises as fs } from 'fs';
import path from 'path';

describe('Claude Code Integration', () => {
  let tempDir: string;
  
  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now());
    await fs.mkdir(tempDir, { recursive: true });
  });
  
  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });
  
  it('should execute claude command and parse events', async () => {
    const agent = new ClaudeCodeAgent('test-task', {
      model: 'claude-3-haiku-20240307', // Use faster model for tests
      maxTokens: 100
    });
    
    const events: any[] = [];
    
    try {
      for await (const event of agent.run('Say hello', { workspaceDir: tempDir })) {
        events.push(event);
        
        // Stop after getting some events to avoid long test
        if (events.length >= 3) {
          await agent.stop();
          break;
        }
      }
    } catch (error) {
      // This test might fail if Claude CLI is not available
      console.log('Claude CLI not available, skipping integration test');
      return;
    }
    
    expect(events.length).toBeGreaterThan(0);
    expect(events[0].type).toBe('start');
  }, 30000); // 30 second timeout
});
```

## Migration Checklist

- [ ] Create base agent interface and abstract class
- [ ] Implement Claude Code agent with subprocess management
- [ ] Add agent options configuration and validation
- [ ] Create agent service for lifecycle management
- [ ] Implement process management utilities
- [ ] Add agent execution tracking and monitoring
- [ ] Create comprehensive error handling
- [ ] Add timeout and resource management
- [ ] Implement agent registry for extensibility
- [ ] Write unit tests for all agent functionality
- [ ] Add integration tests with real Claude CLI
- [ ] Test error scenarios and edge cases
- [ ] Validate performance and resource usage
- [ ] Add monitoring and logging

## Security Considerations

### 1. Process Security
- Validate all command arguments
- Use subprocess isolation
- Implement resource limits (memory, CPU, time)
- Sanitize environment variables

### 2. Agent Execution
- Validate agent options and configuration
- Implement execution timeouts
- Monitor resource usage
- Log all agent activities

### 3. Workspace Access
- Ensure agents can only access their assigned workspace
- Validate file paths and operations
- Prevent privilege escalation
- Monitor file system access

## Performance Optimization

### 1. Process Management
- Reuse processes when possible
- Implement connection pooling
- Use efficient IPC mechanisms
- Monitor and limit resource usage

### 2. Event Streaming
- Use efficient event serialization
- Implement event batching
- Optimize memory usage
- Handle backpressure

### 3. Agent Lifecycle
- Clean up resources promptly
- Implement graceful shutdown
- Monitor agent health
- Use lazy initialization

## Next Steps

1. Review [Utilities Migration](./08-utilities.md)
2. Implement additional agent types (OpenAI, Anthropic API)
3. Add agent monitoring and metrics
4. Set up agent execution sandboxing
5. Configure agent orchestration policies