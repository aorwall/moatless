import { describe, expect, it } from 'vitest'
import { taskWorkspaceLabel } from '../task-workspace-label'

describe('taskWorkspaceLabel', () => {
  it('prefers the workspace name', () => {
    expect(
      taskWorkspaceLabel({
        workspaceName: 'Moatless Vibe',
        repositoryName: 'moatless-vibe',
        repositoryId: 'repo-1',
      }),
    ).toBe('Moatless Vibe')
  })

  it('falls back to the repository name when the workspace could not be named', () => {
    expect(
      taskWorkspaceLabel({
        workspaceName: null,
        repositoryName: 'moatless-vibe',
        repositoryId: 'repo-1',
      }),
    ).toBe('moatless-vibe')
  })

  it('falls back to the repository id when neither name resolved', () => {
    expect(taskWorkspaceLabel({ repositoryId: 'repo-1' })).toBe('repo-1')
  })
})
