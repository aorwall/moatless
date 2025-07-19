#!/usr/bin/env bun

/**
 * Manual test script to verify that the getFileChangeInfo fix works correctly.
 * This tests the actual functionality without complex test infrastructure.
 */

import { execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { getFileChangeInfo } from '../utils/git-utils.js'

async function createTestRepo(): Promise<string> {
  // Create temporary directory outside the main repo
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'file-change-test-'))

  console.log(`\n🔧 Creating test repository in: ${tempDir}`)

  // Initialize git repo without hooks
  execSync('git init', { cwd: tempDir })
  execSync('git config user.name "Test User"', { cwd: tempDir })
  execSync('git config user.email "test@example.com"', { cwd: tempDir })
  execSync('git config core.hooksPath /dev/null', { cwd: tempDir })

  // Create initial commit
  await fs.writeFile(path.join(tempDir, 'README.md'), '# Test Repository\nInitial content')
  execSync('git add README.md', { cwd: tempDir })
  execSync('git commit -m "Initial commit"', { cwd: tempDir })

  console.log('✅ Test repository created successfully')

  return tempDir
}

async function testModifiedFile(repoPath: string): Promise<void> {
  console.log('\n📝 Testing modified file...')

  const fileName = 'test-modified.txt'
  const originalContent = 'Line 1\nLine 2\nLine 3'
  const modifiedContent = 'Modified Line 1\nLine 2\nLine 3\nNew Line 4'

  // Create and commit original file
  await fs.writeFile(path.join(repoPath, fileName), originalContent)
  execSync(`git add "${fileName}"`, { cwd: repoPath })
  execSync('git commit -m "Add test file"', { cwd: repoPath })

  // Modify the file
  await fs.writeFile(path.join(repoPath, fileName), modifiedContent)

  // Test getFileChangeInfo
  const changeInfo = await getFileChangeInfo(repoPath, fileName)

  console.log('📋 Results:')
  console.log(`   - isModified: ${changeInfo.isModified}`)
  console.log(`   - status: ${changeInfo.status}`)
  console.log(`   - additions: ${changeInfo.additions}`)
  console.log(`   - deletions: ${changeInfo.deletions}`)
  console.log(`   - oldContent length: ${changeInfo.oldContent?.length || 0}`)
  console.log(`   - oldContent matches: ${changeInfo.oldContent === originalContent}`)

  // Verify results
  if (
    changeInfo.isModified &&
    changeInfo.status === 'modified' &&
    changeInfo.oldContent === originalContent &&
    changeInfo.additions > 0
  ) {
    console.log('✅ Modified file test PASSED')
  } else {
    console.log('❌ Modified file test FAILED')
    console.log('Expected: isModified=true, status=modified, oldContent to match original')
  }
}

async function testNewFile(repoPath: string): Promise<void> {
  console.log('\n🆕 Testing new file...')

  const fileName = 'test-new.txt'
  const newContent = 'This is a new file\nWith multiple lines\nFor testing'

  // Create new file (untracked)
  await fs.writeFile(path.join(repoPath, fileName), newContent)

  // Test getFileChangeInfo
  const changeInfo = await getFileChangeInfo(repoPath, fileName)

  console.log('📋 Results:')
  console.log(`   - isModified: ${changeInfo.isModified}`)
  console.log(`   - status: ${changeInfo.status}`)
  console.log(`   - additions: ${changeInfo.additions}`)
  console.log(`   - deletions: ${changeInfo.deletions}`)
  console.log(`   - oldContent: ${changeInfo.oldContent}`)

  // Verify results
  if (
    changeInfo.isModified &&
    changeInfo.status === 'added' &&
    changeInfo.oldContent === null &&
    changeInfo.additions > 0 &&
    changeInfo.deletions === 0
  ) {
    console.log('✅ New file test PASSED')
  } else {
    console.log('❌ New file test FAILED')
    console.log(
      'Expected: isModified=true, status=added, oldContent=null, additions>0, deletions=0',
    )
  }
}

async function testDeletedFile(repoPath: string): Promise<void> {
  console.log('\n🗑️ Testing deleted file...')

  const fileName = 'test-deleted.txt'
  const originalContent = 'This file will be deleted\nSecond line\nThird line'

  // Create and commit file
  await fs.writeFile(path.join(repoPath, fileName), originalContent)
  execSync(`git add "${fileName}"`, { cwd: repoPath })
  execSync('git commit -m "Add file to delete"', { cwd: repoPath })

  // Delete the file
  await fs.unlink(path.join(repoPath, fileName))

  // Test getFileChangeInfo
  const changeInfo = await getFileChangeInfo(repoPath, fileName)

  console.log('📋 Results:')
  console.log(`   - isModified: ${changeInfo.isModified}`)
  console.log(`   - status: ${changeInfo.status}`)
  console.log(`   - additions: ${changeInfo.additions}`)
  console.log(`   - deletions: ${changeInfo.deletions}`)
  console.log(`   - oldContent length: ${changeInfo.oldContent?.length || 0}`)
  console.log(`   - oldContent matches: ${changeInfo.oldContent === originalContent}`)

  // Verify results
  if (
    changeInfo.isModified &&
    changeInfo.status === 'deleted' &&
    changeInfo.oldContent === originalContent &&
    changeInfo.additions === 0 &&
    changeInfo.deletions > 0
  ) {
    console.log('✅ Deleted file test PASSED')
  } else {
    console.log('❌ Deleted file test FAILED')
    console.log(
      'Expected: isModified=true, status=deleted, oldContent to match original, additions=0, deletions>0',
    )
  }
}

async function testUnchangedFile(repoPath: string): Promise<void> {
  console.log('\n🔄 Testing unchanged file...')

  const fileName = 'test-unchanged.txt'
  const content = 'This file will not change'

  // Create and commit file
  await fs.writeFile(path.join(repoPath, fileName), content)
  execSync(`git add "${fileName}"`, { cwd: repoPath })
  execSync('git commit -m "Add unchanged file"', { cwd: repoPath })

  // Test getFileChangeInfo (no modifications)
  const changeInfo = await getFileChangeInfo(repoPath, fileName)

  console.log('📋 Results:')
  console.log(`   - isModified: ${changeInfo.isModified}`)
  console.log(`   - status: ${changeInfo.status}`)
  console.log(`   - additions: ${changeInfo.additions}`)
  console.log(`   - deletions: ${changeInfo.deletions}`)
  console.log(`   - oldContent: ${changeInfo.oldContent}`)

  // Verify results
  if (
    !changeInfo.isModified &&
    changeInfo.status === 'unchanged' &&
    changeInfo.oldContent === null &&
    changeInfo.additions === 0 &&
    changeInfo.deletions === 0
  ) {
    console.log('✅ Unchanged file test PASSED')
  } else {
    console.log('❌ Unchanged file test FAILED')
    console.log(
      'Expected: isModified=false, status=unchanged, oldContent=null, additions=0, deletions=0',
    )
  }
}

async function testNonExistentFile(repoPath: string): Promise<void> {
  console.log('\n👻 Testing non-existent file...')

  const fileName = 'does-not-exist.txt'

  // Test getFileChangeInfo for non-existent file
  const changeInfo = await getFileChangeInfo(repoPath, fileName)

  console.log('📋 Results:')
  console.log(`   - isModified: ${changeInfo.isModified}`)
  console.log(`   - status: ${changeInfo.status}`)
  console.log(`   - additions: ${changeInfo.additions}`)
  console.log(`   - deletions: ${changeInfo.deletions}`)
  console.log(`   - oldContent: ${changeInfo.oldContent}`)

  // Verify results
  if (
    !changeInfo.isModified &&
    changeInfo.status === 'unchanged' &&
    changeInfo.oldContent === null &&
    changeInfo.additions === 0 &&
    changeInfo.deletions === 0
  ) {
    console.log('✅ Non-existent file test PASSED')
  } else {
    console.log('❌ Non-existent file test FAILED')
    console.log(
      'Expected: isModified=false, status=unchanged, oldContent=null, additions=0, deletions=0',
    )
  }
}

async function cleanupRepo(repoPath: string): Promise<void> {
  try {
    await fs.rm(repoPath, { recursive: true, force: true })
    console.log(`\n🧹 Cleaned up test repository: ${repoPath}`)
  } catch (error) {
    console.warn(`⚠️ Failed to cleanup: ${error}`)
  }
}

async function main(): Promise<void> {
  console.log('🚀 Testing getFileChangeInfo function fix...')

  let repoPath: string | null = null

  try {
    repoPath = await createTestRepo()

    // Run all tests
    await testModifiedFile(repoPath)
    await testNewFile(repoPath)
    await testDeletedFile(repoPath)
    await testUnchangedFile(repoPath)
    await testNonExistentFile(repoPath)

    console.log('\n🎉 All tests completed!')
    console.log(
      '\n📊 Summary: If all tests show ✅ PASSED, the getFileChangeInfo fix is working correctly.',
    )
    console.log(
      'This means the file tree and individual file content endpoints should now be consistent.',
    )
  } catch (error) {
    console.error('❌ Test failed with error:', error)
    process.exit(1)
  } finally {
    if (repoPath) {
      await cleanupRepo(repoPath)
    }
  }
}

// Run the test
main().catch(console.error)
