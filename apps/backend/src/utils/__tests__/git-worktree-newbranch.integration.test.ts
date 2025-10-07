import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { createGitWorktree, isGitWorktree } from '../index.js'

const execAsync = promisify(exec)

describe('Git Worktree NewBranch Integration Test', () => {
  let testRepoPath: string
  let testWorktreeDir: string

  beforeAll(async () => {
    // Create a test git repository
    const tempDir = process.env['TEMP'] || '/tmp'
    testRepoPath = path.join(tempDir, `test-repo-newbranch-${Date.now()}`)
    testWorktreeDir = path.join(tempDir, `test-worktrees-newbranch-${Date.now()}`)

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

  it('should create new branch from existing local branch when both branchName and newBranchName are provided', async () => {
    const sourceBranch = 'feature/source-branch'
    const newBranch = 'feature/new-branch-from-source'
    const workspacePath = path.join(testWorktreeDir, `workspace-local-${Date.now()}`)

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

    // Verify the content is correct
    const content = await fs.readFile(path.join(workspacePath, 'source-file.txt'), 'utf-8')
    expect(content).toBe('Source branch content')

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
    const workspacePath = path.join(testWorktreeDir, `workspace-remote-${Date.now()}`)

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

    // Rename remote to 'origin' for the test (since our code expects 'origin')
    await execAsync('git remote rename test-origin origin', {
      cwd: testRepoPath,
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

    // Verify the content is correct
    const content = await fs.readFile(path.join(workspacePath, 'remote-source.txt'), 'utf-8')
    expect(content).toBe('Remote source content')

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

  it('should still work correctly when only branchName is provided (backward compatibility)', async () => {
    const branchName = 'feature/existing-branch'
    const workspacePath = path.join(testWorktreeDir, `workspace-compat-${Date.now()}`)

    // Create a branch
    await execAsync(`git checkout -b ${branchName}`, { cwd: testRepoPath })
    await fs.writeFile(path.join(testRepoPath, 'branch-file.txt'), 'Branch content')
    await execAsync('git add .', { cwd: testRepoPath })
    await execAsync('git commit -m "Branch commit"', { cwd: testRepoPath })

    // Switch back to main
    await execAsync('git checkout main', { cwd: testRepoPath })

    // Create worktree with only branchName (no newBranchName)
    await createGitWorktree(testRepoPath, workspacePath, branchName)

    // Verify worktree was created
    const isValid = await isGitWorktree(workspacePath)
    expect(isValid).toBe(true)

    // Verify it's on the original branch
    const { stdout: currentBranch } = await execAsync('git branch --show-current', {
      cwd: workspacePath,
    })
    expect(currentBranch.trim()).toBe(branchName)

    // Verify the file exists
    const fileExists = await fs
      .access(path.join(workspacePath, 'branch-file.txt'))
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)

    // Clean up
    await execAsync(`git worktree remove "${workspacePath}" --force`, {
      cwd: testRepoPath,
    }).catch(() => {
      /* ignore cleanup errors */
    })
    await execAsync(`git branch -D ${branchName}`, { cwd: testRepoPath }).catch(() => {
      /* ignore cleanup errors */
    })
  })
})
