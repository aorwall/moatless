import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { CreateRepositoryRequest, CreateTaskRequest } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'
import { getDataDir } from '../../utils/data-dir.js'

/**
 * Backend Integration Test for Upload Workflow with Workspace
 *
 * This test verifies the complete file upload workflow:
 * 1. Create a real git repository and repository config
 * 2. Upload files to temp directory
 * 3. Create task without starting it (to avoid agent execution)
 * 4. Verify files are moved from temp to task directory
 * 5. Verify files are copied to workspace .uploads directory
 * 6. Test file retrieval from both locations
 * 7. Cleanup verification
 */

describe('Upload Integration Test with Workspace', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let testDataDir: string
  let testUploadDir: string

  beforeEach(async () => {
    app = await build()

    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-upload-test-'))

    // Set up test-specific upload directory
    testDataDir = getDataDir()
    testUploadDir = path.join(testDataDir, 'uploads')

    // Ensure clean state
    try {
      await fs.rm(testUploadDir, { recursive: true, force: true })
    } catch {
      // Directory might not exist
    }
    await fs.mkdir(testUploadDir, { recursive: true })

    // Initialize a real git repository in the temp directory
    await initializeTestRepository(tempDir)

    // Create repository configuration
    const createRepoRequest: CreateRepositoryRequest = {
      name: 'Test Upload Repository',
      description: 'Repository for upload integration testing',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
      },
    }

    const repoResponse = await app.fetch(
      new Request('http://localhost/api/v1/repositories', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(createRepoRequest),
      }),
    )

    expect(repoResponse.status).toBe(200)
    const repository = await repoResponse.json()
    createdRepositoryId = repository.id
  })

  afterEach(async () => {
    // Cleanup task if created
    if (createdTaskId) {
      try {
        await app.fetch(
          new Request(`http://localhost/api/v1/tasks/${createdTaskId}`, {
            method: 'DELETE',
          }),
        )
      } catch {
        // Task might already be cleaned up
      }
    }

    // Cleanup repository if created
    if (createdRepositoryId) {
      try {
        await app.fetch(
          new Request(`http://localhost/api/v1/repositories/${createdRepositoryId}`, {
            method: 'DELETE',
          }),
        )
      } catch {
        // Repository might already be cleaned up
      }
    }

    // Clean up temporary directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch {
      // Directory might already be cleaned up
    }

    // Clean up test upload directory
    try {
      await fs.rm(testUploadDir, { recursive: true, force: true })
    } catch {
      // Directory might not exist
    }
  })

  it('should complete full upload workflow with Workspace', async () => {
    // Step 1: Upload files to temp directory
    const textFileContent = 'Hello, this is a test document for upload workflow testing.'
    const imageFileContent = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) // PNG header
    const jsonFileContent = JSON.stringify({ message: 'Test JSON data', timestamp: Date.now() })

    const uploadedFiles = []

    // Upload text file
    const textFormData = new FormData()
    const textFile = new File([textFileContent], 'test-document.txt', { type: 'text/plain' })
    textFormData.append('file', textFile)

    const textUploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: textFormData,
      }),
    )
    expect(textUploadResponse.status).toBe(200)
    const textUploadResult = await textUploadResponse.json()
    uploadedFiles.push(textUploadResult.absolutePath)

    // Upload image file
    const imageFormData = new FormData()
    const imageFile = new File([imageFileContent], 'test-image.png', { type: 'image/png' })
    imageFormData.append('file', imageFile)

    const imageUploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: imageFormData,
      }),
    )
    expect(imageUploadResponse.status).toBe(200)
    const imageUploadResult = await imageUploadResponse.json()
    uploadedFiles.push(imageUploadResult.absolutePath)

    // Upload JSON file
    const jsonFormData = new FormData()
    const jsonFile = new File([jsonFileContent], 'test-data.json', { type: 'application/json' })
    jsonFormData.append('file', jsonFile)

    const jsonUploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: jsonFormData,
      }),
    )
    expect(jsonUploadResponse.status).toBe(200)
    const jsonUploadResult = await jsonUploadResponse.json()
    uploadedFiles.push(jsonUploadResult.absolutePath)

    // Verify all files were uploaded to temp directories
    expect(textUploadResult.id).toMatch(/^temp-/)
    expect(imageUploadResult.id).toMatch(/^temp-/)
    expect(jsonUploadResult.id).toMatch(/^temp-/)

    // Verify files exist in temp directories
    for (const filePath of uploadedFiles) {
      const fileExists = await fs
        .access(filePath)
        .then(() => true)
        .catch(() => false)
      expect(fileExists).toBe(true)
    }

    // Step 2: Create task without starting it (start: false)
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: createdRepositoryId,
      message: 'Process uploaded files',
      start: false, // Don't start the agent to avoid execution
      filePaths: uploadedFiles,
    }

    const taskResponse = await app.fetch(
      new Request('http://localhost/api/v1/tasks', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(createTaskRequest),
      }),
    )

    expect(taskResponse.status).toBe(200)
    const taskResult = await taskResponse.json()
    createdTaskId = taskResult.taskId

    // Step 3: Get task to verify file attachments were processed
    const getTaskResponse = await app.fetch(
      new Request(`http://localhost/api/v1/tasks/${createdTaskId}`, {
        method: 'GET',
      }),
    )

    expect(getTaskResponse.status).toBe(200)
    const task = await getTaskResponse.json()

    // Verify task has iterations with attachments
    expect(task.iterations).toHaveLength(1)
    const iteration = task.iterations[0]
    expect(iteration.attachments).toHaveLength(3)

    // Verify attachments have correct metadata
    const textAttachment = iteration.attachments.find(
      (a: any) => a.filename === 'test-document.txt',
    )
    const imageAttachment = iteration.attachments.find((a: any) => a.filename === 'test-image.png')
    const jsonAttachment = iteration.attachments.find((a: any) => a.filename === 'test-data.json')

    expect(textAttachment).toBeDefined()
    expect(textAttachment.mimeType).toBe('text/plain')
    expect(imageAttachment).toBeDefined()
    expect(imageAttachment.mimeType).toBe('image/png')
    expect(jsonAttachment).toBeDefined()
    expect(jsonAttachment.mimeType).toBe('application/json')

    // Step 4: Verify files were moved from temp to task directory
    const taskUploadDir = path.join(testUploadDir, createdTaskId)
    const taskDirExists = await fs
      .access(taskUploadDir)
      .then(() => true)
      .catch(() => false)
    expect(taskDirExists).toBe(true)

    const taskFiles = await fs.readdir(taskUploadDir)
    expect(taskFiles).toContain('test-document.txt')
    expect(taskFiles).toContain('test-image.png')
    expect(taskFiles).toContain('test-data.json')

    // Verify file contents in task directory
    const taskTextContent = await fs.readFile(
      path.join(taskUploadDir, 'test-document.txt'),
      'utf-8',
    )
    expect(taskTextContent).toBe(textFileContent)

    const taskImageContent = await fs.readFile(path.join(taskUploadDir, 'test-image.png'))
    expect(new Uint8Array(taskImageContent)).toEqual(imageFileContent)

    const taskJsonContent = await fs.readFile(path.join(taskUploadDir, 'test-data.json'), 'utf-8')
    expect(taskJsonContent).toBe(jsonFileContent)

    // Step 5: Test workspace integration - get workspace status to ensure workspace was created
    const workspaceStatusResponse = await app.fetch(
      new Request(`http://localhost/api/v1/tasks/${createdTaskId}/workspace/status`, {
        method: 'GET',
      }),
    )

    expect(workspaceStatusResponse.status).toBe(200)
    const workspaceStatus = await workspaceStatusResponse.json()
    expect(workspaceStatus.status).toBeDefined()

    // Step 6: Test file retrieval via upload endpoints
    // Retrieve text file
    const retrieveTextResponse = await app.fetch(
      new Request(`http://localhost/api/v1/uploads/${createdTaskId}/test-document.txt`, {
        method: 'GET',
      }),
    )
    expect(retrieveTextResponse.status).toBe(200)
    expect(retrieveTextResponse.headers.get('content-type')).toBe('text/plain')
    const retrievedTextContent = await retrieveTextResponse.text()
    expect(retrievedTextContent).toBe(textFileContent)

    // Retrieve image file
    const retrieveImageResponse = await app.fetch(
      new Request(`http://localhost/api/v1/uploads/${createdTaskId}/test-image.png`, {
        method: 'GET',
      }),
    )
    expect(retrieveImageResponse.status).toBe(200)
    expect(retrieveImageResponse.headers.get('content-type')).toBe('image/png')
    expect(retrieveImageResponse.headers.get('cache-control')).toBe('public, max-age=31536000')
    const retrievedImageContent = new Uint8Array(await retrieveImageResponse.arrayBuffer())
    expect(retrievedImageContent).toEqual(imageFileContent)

    // Retrieve JSON file
    const retrieveJsonResponse = await app.fetch(
      new Request(`http://localhost/api/v1/uploads/${createdTaskId}/test-data.json`, {
        method: 'GET',
      }),
    )
    expect(retrieveJsonResponse.status).toBe(200)
    expect(retrieveJsonResponse.headers.get('content-type')).toBe('application/json')
    const retrievedJsonContent = await retrieveJsonResponse.text()
    expect(retrievedJsonContent).toBe(jsonFileContent)

    // Step 7: Test copying uploads to workspace (if workspace path is available)
    if (task.workspacePath) {
      // This would be tested when agent actually starts and copies files to workspace
      // For now, verify that copyUploadsToWorkspace method would work
      const attachmentPaths = iteration.attachments.map((a: any) => a.path)

      // Make a direct call to workspace service to test file copying
      const copiedPaths = await app.services.workspace.copyUploadsToWorkspace(
        createdTaskId,
        attachmentPaths,
      )
      expect(copiedPaths).toHaveLength(3)

      // Verify files were copied to workspace .uploads directory
      const workspaceUploadsDir = path.join(task.workspacePath, '.uploads')
      const workspaceUploadsDirExists = await fs
        .access(workspaceUploadsDir)
        .then(() => true)
        .catch(() => false)
      expect(workspaceUploadsDirExists).toBe(true)

      const workspaceFiles = await fs.readdir(workspaceUploadsDir)
      expect(workspaceFiles).toContain('test-document.txt')
      expect(workspaceFiles).toContain('test-image.png')
      expect(workspaceFiles).toContain('test-data.json')

      // Verify file contents in workspace
      const workspaceTextContent = await fs.readFile(
        path.join(workspaceUploadsDir, 'test-document.txt'),
        'utf-8',
      )
      expect(workspaceTextContent).toBe(textFileContent)
    }

    // Step 8: Verify temp directories were cleaned up
    const tempDirs = await fs.readdir(path.join(testUploadDir, 'temp')).catch(() => [])
    expect(tempDirs.length).toBe(0) // All temp directories should be cleaned up after moving files
  })

  it('should handle upload errors gracefully in integration', async () => {
    // Test uploading a file directly to task directory when task doesn't exist
    const testContent = 'Test content for non-existent task'
    const fakeTaskId = randomUUID()

    const formData = new FormData()
    const testFile = new File([testContent], 'test.txt', { type: 'text/plain' })
    formData.append('file', testFile)
    formData.append('taskId', fakeTaskId)

    const uploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: formData,
      }),
    )

    // Upload should succeed even if task doesn't exist yet
    expect(uploadResponse.status).toBe(200)
    const uploadResult = await uploadResponse.json()
    expect(uploadResult.id).toBe(fakeTaskId)

    // Verify file was created in task directory
    const taskDir = path.join(testUploadDir, fakeTaskId)
    const fileExists = await fs
      .access(path.join(taskDir, 'test.txt'))
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)

    // Clean up test file
    await fs.rm(taskDir, { recursive: true, force: true })
  })

  it('should handle temp file cleanup', async () => {
    // Upload a file to temp directory
    const testContent = 'Temp file for cleanup test'

    const formData = new FormData()
    const testFile = new File([testContent], 'temp-cleanup-test.txt', { type: 'text/plain' })
    formData.append('file', testFile)

    const uploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: formData,
      }),
    )

    expect(uploadResponse.status).toBe(200)
    const uploadResult = await uploadResponse.json()

    // Verify file exists in temp directory
    const tempFileExists = await fs
      .access(uploadResult.absolutePath)
      .then(() => true)
      .catch(() => false)
    expect(tempFileExists).toBe(true)

    // Test temp file cleanup
    const cleanedCount = await app.services.task.cleanupOldTempFiles(0) // Clean files older than 0 hours (all files)
    expect(cleanedCount).toBeGreaterThanOrEqual(1)

    // Verify temp file was cleaned up
    const tempFileExistsAfterCleanup = await fs
      .access(uploadResult.absolutePath)
      .then(() => true)
      .catch(() => false)
    expect(tempFileExistsAfterCleanup).toBe(false)
  })
})

/**
 * Initialize a test git repository
 */
async function initializeTestRepository(repositoryPath: string): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  // Initialize git repository
  await execAsync('git init', { cwd: repositoryPath })
  await execAsync('git config user.email "test@example.com"', { cwd: repositoryPath })
  await execAsync('git config user.name "Test User"', { cwd: repositoryPath })

  // Create a basic file structure
  await fs.writeFile(
    path.join(repositoryPath, 'README.md'),
    '# Test Repository for Upload Integration\n\nThis is a test repository.',
  )
  await fs.writeFile(
    path.join(repositoryPath, 'package.json'),
    JSON.stringify(
      {
        name: 'test-upload-repo',
        version: '1.0.0',
        description: 'Test repository for upload integration testing',
        scripts: {
          dev: 'echo "Development server"',
          start: 'echo "Production server"',
        },
      },
      null,
      2,
    ),
  )

  // Create initial commit
  await execAsync('git add .', { cwd: repositoryPath })
  await execAsync('git commit -m "Initial commit"', { cwd: repositoryPath })
}
