import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { branchExists, createGitWorktree, isGitWorktree } from '../index'

const execAsync = promisify(exec)

describe('Git Worktree Fix Test', () => {
  let testRepoPath: string
  let testWorktreeDir: string
  let testTaskId: string

  beforeAll(async () => {
    // Create a test git repository
    const tempDir = process.env['TEMP'] || '/tmp'
    testRepoPath = path.join(tempDir, `test-repo-${Date.now()}`)
    testWorktreeDir = path.join(tempDir, `test-worktrees-${Date.now()}`)
    testTaskId = `test-task-${Date.now()}`

    // Initialize git repo
    await fs.mkdir(testRepoPath, { recursive: true })
    await execAsync('git init', { cwd: testRepoPath })
    await execAsync('git config user.email "test@example.com"', {
      cwd: testRepoPath,
    })
    await execAsync('git config user.name "Test User"', { cwd: testRepoPath })

    // Create initial commit
    await fs.writeFile(path.join(testRepoPath, 'README.md'), '# Test Repo')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Initial commit"', { cwd: testRepoPath })
  })

  afterAll(async () => {
    // Clean up test directories
    await fs.rm(testRepoPath, { recursive: true, force: true }).catch(() => {
      /* ignore cleanup errors */
    })
    await fs.rm(testWorktreeDir, { recursive: true, force: true }).catch(() => {
      /* ignore cleanup errors */
    })
  })

  it('should handle existing branch when creating worktree', async () => {
    const workspacePath = path.join(testWorktreeDir, `workspace-${testTaskId}`)
    const branchName = `workspace-${testTaskId}`

    // Create worktree with new branch
    await createGitWorktree(testRepoPath, workspacePath, undefined, branchName)

    // Verify worktree exists
    const isValid1 = await isGitWorktree(workspacePath)
    expect(isValid1).toBe(true)

    // Remove the worktree but keep the branch
    await execAsync(`git worktree remove "${workspacePath}" --force`, {
      cwd: testRepoPath,
    })

    // Verify directory is gone
    const dirExists = await fs
      .access(workspacePath)
      .then(() => true)
      .catch(() => false)
    expect(dirExists).toBe(false)

    // Try to create worktree again with same branch name - should use existing branch
    await createGitWorktree(testRepoPath, workspacePath, branchName)

    // Verify worktree was recreated
    const isValid2 = await isGitWorktree(workspacePath)
    expect(isValid2).toBe(true)

    // Verify it's using the correct branch
    const { stdout } = await execAsync('git branch --show-current', {
      cwd: workspacePath,
    })
    expect(stdout.trim()).toBe(branchName)
  })

  it('should validate worktree correctly', async () => {
    const workspacePath = path.join(testWorktreeDir, `workspace-validate-${Date.now()}`)
    const branchName = `workspace-validate-${Date.now()}`

    // Create worktree
    await createGitWorktree(testRepoPath, workspacePath, undefined, branchName)

    // Should be valid
    const isValid = await isGitWorktree(workspacePath)
    expect(isValid).toBe(true)

    // Create a regular directory (not a worktree)
    const regularDir = path.join(testWorktreeDir, 'regular-dir')
    await fs.mkdir(regularDir, { recursive: true })

    // Should not be valid
    const isValidRegular = await isGitWorktree(regularDir)
    expect(isValidRegular).toBe(false)
  })

  it('should create new branch from existing local branch when both branchName and newBranchName are provided', async () => {
    const sourceBranch = 'feature/source-branch'
    const newBranch = 'feature/new-branch-from-source'
    const workspacePath = path.join(testWorktreeDir, `workspace-newbranch-${Date.now()}`)

    // Create source branch with some commits
    await execAsync(`git checkout -b ${sourceBranch}`, { cwd: testRepoPath })
    await fs.writeFile(path.join(testRepoPath, 'source-file.txt'), 'Source branch content')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Source branch commit"', {
      cwd: testRepoPath,
    })

    // Switch back to main so source branch is not checked out
    await execAsync('git checkout main', { cwd: testRepoPath })

    // Create worktree with new branch based on source branch
    await createGitWorktree(testRepoPath, workspacePath, sourceBranch, newBranch)

    // Verify worktree was created
    const isValid = await isGitWorktree(workspacePath)
    expect(isValid).toBe(true)

    // Verify it's on the NEW branch, not the source branch
    const { stdout: currentBranch } = await execAsync('git branch --show-current', {
      cwd: workspacePath,
    })
    expect(currentBranch.trim()).toBe(newBranch)

    // Verify the file from source branch exists in new branch
    const fileExists = await fs
      .access(path.join(workspacePath, 'source-file.txt'))
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)

    // Clean up
    await execAsync(`git worktree remove "${workspacePath}" --force`, {
      cwd: testRepoPath,
    }).catch(() => {
      /* ignore cleanup errors */
    })
    await execAsync(`git branch -D ${sourceBranch}`, {
      cwd: testRepoPath,
    }).catch(() => {
      /* ignore cleanup errors */
    })
    await execAsync(`git branch -D ${newBranch}`, { cwd: testRepoPath }).catch(() => {
      /* ignore cleanup errors */
    })
  })

  it('should create new branch from remote branch when both branchName and newBranchName are provided', async () => {
    const remoteBranchName = 'feature/remote-source'
    const newBranch = 'feature/new-from-remote'
    const workspacePath = path.join(testWorktreeDir, `workspace-remote-new-${Date.now()}`)

    // Create a bare repository to act as remote
    const remoteRepoPath = path.join(testWorktreeDir, `remote-repo-${Date.now()}.git`)
    await execAsync(`git init --bare ${remoteRepoPath}`)

    // Add remote to test repo
    await execAsync(`git remote add test-origin ${remoteRepoPath}`, {
      cwd: testRepoPath,
    })

    // Create and push a branch to remote
    await execAsync(`git checkout -b ${remoteBranchName}`, {
      cwd: testRepoPath,
    })
    await fs.writeFile(path.join(testRepoPath, 'remote-source.txt'), 'Remote source content')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Remote source commit"', {
      cwd: testRepoPath,
    })
    await execAsync(`git push test-origin ${remoteBranchName}`, {
      cwd: testRepoPath,
    })

    // Switch back to main and delete the local branch
    await execAsync('git checkout main', { cwd: testRepoPath })
    await execAsync(`git branch -D ${remoteBranchName}`, { cwd: testRepoPath })

    // Update remote refs
    await execAsync('git fetch test-origin', { cwd: testRepoPath })

    // Set the remote as origin temporarily for the test
    await execAsync('git remote rename test-origin origin', {
      cwd: testRepoPath,
    }).catch(() => {
      /* ignore cleanup errors */
    })

    // Create worktree with new branch based on remote branch
    await createGitWorktree(testRepoPath, workspacePath, remoteBranchName, newBranch)

    // Verify worktree was created
    const isValid = await isGitWorktree(workspacePath)
    expect(isValid).toBe(true)

    // Verify it's on the NEW branch, not the remote branch name
    const { stdout: currentBranch } = await execAsync('git branch --show-current', {
      cwd: workspacePath,
    })
    expect(currentBranch.trim()).toBe(newBranch)

    // Verify the file from remote branch exists in new branch
    const fileExists = await fs
      .access(path.join(workspacePath, 'remote-source.txt'))
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)

    // Clean up
    await execAsync(`git worktree remove "${workspacePath}" --force`, {
      cwd: testRepoPath,
    }).catch(() => {
      /* ignore cleanup errors */
    })
    await execAsync(`git branch -D ${newBranch}`, { cwd: testRepoPath }).catch(() => {
      /* ignore cleanup errors */
    })
    await execAsync('git remote remove origin', { cwd: testRepoPath }).catch(() => {
      /* ignore cleanup errors */
    })
    await fs.rm(remoteRepoPath, { recursive: true, force: true }).catch(() => {
      /* ignore cleanup errors */
    })
  })

  it('should handle remote-only branches when creating worktree', async () => {
    const workspacePath = path.join(testWorktreeDir, `workspace-remote-${Date.now()}`)
    const remoteBranchName = 'feature/remote-only-branch'

    // Create a bare repository to act as remote
    const remoteRepoPath = path.join(testWorktreeDir, 'remote-repo.git')
    await execAsync(`git init --bare ${remoteRepoPath}`)

    // Add remote to test repo
    await execAsync(`git remote add origin ${remoteRepoPath}`, {
      cwd: testRepoPath,
    })

    // Create and push a branch to remote
    await execAsync(`git checkout -b ${remoteBranchName}`, {
      cwd: testRepoPath,
    })
    await fs.writeFile(path.join(testRepoPath, 'remote-file.txt'), 'Remote content')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Remote branch commit"', {
      cwd: testRepoPath,
    })
    await execAsync(`git push origin ${remoteBranchName}`, {
      cwd: testRepoPath,
    })

    // Switch back to main and delete the local branch
    await execAsync('git checkout main', { cwd: testRepoPath })
    await execAsync(`git branch -D ${remoteBranchName}`, { cwd: testRepoPath })

    // Verify the branch doesn't exist locally but exists on remote
    const { stdout: localBranches } = await execAsync('git branch', {
      cwd: testRepoPath,
    })
    expect(localBranches).not.toContain(remoteBranchName)

    const { stdout: remoteBranches } = await execAsync('git branch -r', {
      cwd: testRepoPath,
    })
    expect(remoteBranches).toContain(`origin/${remoteBranchName}`)

    // Test the branchExists function recognizes remote branches
    const exists = await branchExists(testRepoPath, remoteBranchName)
    expect(exists).toBe(true)

    // Create worktree from remote-only branch
    await createGitWorktree(testRepoPath, workspacePath, remoteBranchName)

    // Verify worktree was created successfully
    const isValid = await isGitWorktree(workspacePath)
    expect(isValid).toBe(true)

    // Verify it's on the correct branch
    const { stdout: currentBranch } = await execAsync('git branch --show-current', {
      cwd: workspacePath,
    })
    expect(currentBranch.trim()).toBe(remoteBranchName)

    // Verify the file from remote branch exists
    const fileExists = await fs
      .access(path.join(workspacePath, 'remote-file.txt'))
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)

    // Clean up remote
    await execAsync('git remote remove origin', { cwd: testRepoPath }).catch(() => {
      /* ignore cleanup errors */
    })
    await fs.rm(remoteRepoPath, { recursive: true, force: true }).catch(() => {
      /* ignore cleanup errors */
    })
  })
})
