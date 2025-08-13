import type {
  GitDiffFile,
  GitDiffResponse,
  GitStatus,
  TaskIteration,
  TaskResponse,
} from '../../types/index.js'

export function createMockTask(overrides?: Partial<TaskResponse>): TaskResponse {
  return {
    taskId: 'test-task-123',
    createdAt: new Date('2024-01-01T00:00:00Z'),
    lastMessageAt: new Date('2024-01-01T01:00:00Z'),
    agentType: 'claude-code',
    repositoryId: 'test-repo',
    status: 'open',
    numTurns: 0,
    totalCostUsd: 0.0,
    iterations: [],
    workspacePath: '/tmp/test-workspace',
    ...overrides,
  }
}

export function createMockTaskIteration(overrides?: Partial<TaskIteration>): TaskIteration {
  return {
    iterationNumber: 1,
    startTime: new Date('2024-01-01T00:00:00Z'),
    endTime: new Date('2024-01-01T00:05:00Z'),
    status: 'completed',
    messageCount: 2,
    costUsd: 0.05,
    contextTokens: 1000,
    initialMessage: 'Test message',
    ...overrides,
  }
}

export function createMockTaskWithIterations(iterationCount = 1): TaskResponse {
  const iterations = Array.from({ length: iterationCount }, (_, index) =>
    createMockTaskIteration({
      iterationNumber: index + 1,
      startTime: new Date(`2024-01-01T0${index}:00:00Z`),
      endTime: new Date(`2024-01-01T0${index}:05:00Z`),
    }),
  )

  return createMockTask({
    iterations,
    numTurns: iterationCount,
    totalCostUsd: iterationCount * 0.05,
  })
}

export function createMockGitDiffFile(overrides?: Partial<GitDiffFile>): GitDiffFile {
  return {
    oldFile: 'src/test.ts',
    newFile: 'src/test.ts',
    hunks: ['@@ -1,3 +1,4 @@\n function test() {\n+  console.log("hello");\n   return true;\n }'],
    ...overrides,
  }
}

export function createMockGitDiffResponse(fileCount = 1): GitDiffResponse {
  const files = Array.from({ length: fileCount }, (_, index) =>
    createMockGitDiffFile({
      oldFile: `src/file${index + 1}.ts`,
      newFile: `src/file${index + 1}.ts`,
    }),
  )

  return {
    diff: files.map(() => '+ added line\n- removed line').join('\n'),
    files,
  }
}

export function createMockGitStatus(overrides?: Partial<GitStatus>): GitStatus {
  return {
    currentBranch: 'feature/test',
    baseBranch: 'main',
    hasUncommittedChanges: false,
    hasConflicts: false,
    status: 'clean',
    commitsAhead: 1,
    commitsBehind: 0,
    originExists: true,
    additions: 10,
    deletions: 2,
    modifications: 3,
    ...overrides,
  }
}
