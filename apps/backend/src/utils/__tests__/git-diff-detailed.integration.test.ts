import { execSync } from 'node:child_process'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getGitDiffDetailed } from '../git-utils.js'

// Helper class for creating and managing test git repositories
class GitTestRepo {
  private tempDir: string

  constructor() {
    // Create a unique temporary directory for this test
    this.tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'git-diff-test-'))
  }

  get path(): string {
    return this.tempDir
  }

  exec(command: string): string {
    return execSync(command, { cwd: this.tempDir, encoding: 'utf8' })
  }

  init(): void {
    this.exec('git init')
    this.exec('git config user.email "test@example.com"')
    this.exec('git config user.name "Test User"')
  }

  createFile(filename: string, content: string): void {
    const filepath = path.join(this.tempDir, filename)
    const dir = path.dirname(filepath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(filepath, content)
  }

  addFile(filename: string): void {
    this.exec(`git add "${filename}"`)
  }

  commit(message: string): void {
    this.exec(`git commit -m "${message}"`)
  }

  createBranch(branchName: string): void {
    this.exec(`git branch "${branchName}"`)
  }

  checkout(branchName: string): void {
    this.exec(`git checkout "${branchName}"`)
  }

  cleanup(): void {
    if (fs.existsSync(this.tempDir)) {
      fs.rmSync(this.tempDir, { recursive: true, force: true })
    }
  }
}

describe('getGitDiffDetailed', () => {
  let repo: GitTestRepo

  beforeEach(() => {
    repo = new GitTestRepo()
  })

  afterEach(() => {
    repo.cleanup()
  })

  it('should return detailed diff with file contents for new files', async () => {
    repo.init()

    // Create initial commit
    repo.createFile('README.md', '# Test Project\n')
    repo.addFile('README.md')
    repo.commit('Initial commit')

    // Create feature branch
    repo.createBranch('feature')
    repo.checkout('feature')

    // Add new files
    repo.createFile('src/index.js', 'console.log("Hello World");\n')
    // This file has 3 lines of code but the way fs.writeFileSync writes it might add an extra line
    repo.createFile('src/utils.js', 'export function add(a, b) {\n  return a + b;\n}')
    repo.addFile('src/index.js')
    // utils.js is untracked

    const result = await getGitDiffDetailed(repo.path, 'main')

    expect(result.files).toHaveLength(2)

    // Check staged new file
    const indexFile = result.files.find((f) => f.filename === 'src/index.js')
    expect(indexFile).toBeDefined()
    expect(indexFile?.status).toBe('added')
    expect(indexFile?.isStaged).toBe(true)
    expect(indexFile?.additions).toBe(1)
    expect(indexFile?.deletions).toBe(0)
    expect(indexFile?.newContent).toBe('console.log("Hello World");\n')
    expect(indexFile?.oldContent).toBeUndefined()

    // Check untracked file
    const utilsFile = result.files.find((f) => f.filename === 'src/utils.js')
    expect(utilsFile).toBeDefined()
    expect(utilsFile?.status).toBe('added')
    expect(utilsFile?.isStaged).toBe(false)

    // Check the actual content first - without trailing newline now
    expect(utilsFile?.newContent).toBe('export function add(a, b) {\n  return a + b;\n}')

    // The file has 3 lines
    expect(utilsFile?.additions).toBe(3)
    expect(utilsFile?.deletions).toBe(0)
    expect(utilsFile?.oldContent).toBeUndefined()

    // Total should be 1 (from index.js) + 3 (from utils.js) = 4
    expect(result.totalAdditions).toBe(4)
    expect(result.totalDeletions).toBe(0)
    expect(result.totalChanges).toBe(2)
  })

  it('should return detailed diff for modified files', async () => {
    repo.init()

    // Create initial files
    repo.createFile('app.js', 'const version = "1.0.0";\nconsole.log(version);\n')
    repo.addFile('app.js')
    repo.commit('Initial commit')

    // Create feature branch
    repo.createBranch('feature')
    repo.checkout('feature')

    // Modify file
    repo.createFile(
      'app.js',
      `const version = "2.0.0";\nconsole.log(\`Version: \${version}\`);\nconsole.log("Updated");\n`,
    )
    repo.addFile('app.js')

    const result = await getGitDiffDetailed(repo.path, 'main')

    expect(result.files).toHaveLength(1)

    const appFile = result.files[0]
    if (!appFile) throw new Error('Expected appFile')
    expect(appFile.status).toBe('modified')
    expect(appFile.filename).toBe('app.js')
    expect(appFile.isStaged).toBe(true)
    expect(appFile.additions).toBe(3) // All lines changed
    expect(appFile.deletions).toBe(2) // Original 2 lines removed
    expect(appFile.oldContent).toBeUndefined() // We don't fetch content for modified files
    expect(appFile.newContent).toBeUndefined()
  })

  it('should return detailed diff for deleted files', async () => {
    repo.init()

    // Create initial files
    repo.createFile('deprecated.js', 'console.log("This will be deleted");\n')
    repo.addFile('deprecated.js')
    repo.commit('Initial commit')

    // Create feature branch
    repo.createBranch('feature')
    repo.checkout('feature')

    // Delete file
    repo.exec('git rm deprecated.js')

    const result = await getGitDiffDetailed(repo.path, 'main')

    expect(result.files).toHaveLength(1)

    const deletedFile = result.files[0]
    if (!deletedFile) throw new Error('Expected deletedFile')
    expect(deletedFile.status).toBe('deleted')
    expect(deletedFile.filename).toBe('deprecated.js')
    expect(deletedFile.isStaged).toBe(true)
    expect(deletedFile.additions).toBe(0)
    expect(deletedFile.deletions).toBe(1)
    expect(deletedFile.oldContent).toBe('console.log("This will be deleted");\n')
    expect(deletedFile.newContent).toBeUndefined()
  })

  it('should return detailed diff with mix of staged and unstaged changes', async () => {
    repo.init()

    // Create initial files
    repo.createFile('config.json', '{"version": "1.0.0"}\n')
    repo.createFile('package.json', '{"name": "test"}\n')
    repo.addFile('config.json')
    repo.addFile('package.json')
    repo.commit('Initial commit')

    // Create feature branch
    repo.createBranch('feature')
    repo.checkout('feature')

    // Stage a modification
    repo.createFile('config.json', '{"version": "2.0.0", "updated": true}\n')
    repo.addFile('config.json')

    // Unstaged modification
    repo.createFile('package.json', '{"name": "test", "version": "2.0.0"}\n')

    // New unstaged file
    repo.createFile('notes.txt', 'TODO: Update documentation\n')

    const result = await getGitDiffDetailed(repo.path, 'main')

    expect(result.files).toHaveLength(3)

    // Check staged modification
    const configFile = result.files.find((f) => f.filename === 'config.json')
    expect(configFile?.status).toBe('modified')
    expect(configFile?.isStaged).toBe(true)

    // Check unstaged modification
    const packageFile = result.files.find((f) => f.filename === 'package.json')
    expect(packageFile?.status).toBe('modified')
    expect(packageFile?.isStaged).toBe(false)

    // Check untracked file
    const notesFile = result.files.find((f) => f.filename === 'notes.txt')
    expect(notesFile?.status).toBe('added')
    expect(notesFile?.isStaged).toBe(false)
    expect(notesFile?.newContent).toBe('TODO: Update documentation\n')
  })

  it('should handle empty repository gracefully', async () => {
    repo.init()

    const result = await getGitDiffDetailed(repo.path)

    expect(result.files).toHaveLength(0)
    expect(result.totalChanges).toBe(0)
    expect(result.totalAdditions).toBe(0)
    expect(result.totalDeletions).toBe(0)
    expect(result.diffString).toBe('')
  })

  it('should handle binary files', async () => {
    repo.init()

    // Create initial commit
    repo.createFile('text.txt', 'text')
    repo.addFile('text.txt')
    repo.commit('Initial commit')

    // Create feature branch
    repo.createBranch('feature')
    repo.checkout('feature')

    // Create a binary file
    const binaryPath = path.join(repo.path, 'binary.dat')
    const binaryData = Buffer.alloc(1024)
    for (let i = 0; i < binaryData.length; i++) {
      binaryData[i] = Math.floor(Math.random() * 256)
    }
    fs.writeFileSync(binaryPath, binaryData)

    const result = await getGitDiffDetailed(repo.path, 'main')

    // Should include the binary file as untracked
    const binaryFile = result.files.find((f) => f.filename === 'binary.dat')
    expect(binaryFile).toBeDefined()
    expect(binaryFile?.status).toBe('added')
    expect(binaryFile?.isStaged).toBe(false)
    // Binary content will be read but may not be meaningful as text
    expect(binaryFile?.newContent).toBeDefined()
  })
})
