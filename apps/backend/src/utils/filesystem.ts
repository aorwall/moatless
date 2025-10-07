import { promises as fs } from 'node:fs'

/**
 * Check if a path exists on the filesystem
 * @param filePath Path to check
 * @returns True if the path exists, false otherwise
 */
export async function pathExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath)
    return true
  } catch {
    return false
  }
}

/**
 * Read a specific range of bytes from a file
 * @param filePath Path to the file
 * @param start Starting byte position
 * @param length Number of bytes to read
 * @returns The content as a string
 */
export async function readFileRange(
  filePath: string,
  start: number,
  length: number,
): Promise<string> {
  const buffer = Buffer.alloc(length)
  const fileHandle = await fs.open(filePath, 'r')

  try {
    const { bytesRead } = await fileHandle.read(buffer, 0, length, start)
    return buffer.subarray(0, bytesRead).toString('utf-8')
  } finally {
    await fileHandle.close()
  }
}

/**
 * Create a directory recursively if it doesn't exist
 * @param dirPath Directory path to create
 */
export async function ensureDirectory(dirPath: string): Promise<void> {
  await fs.mkdir(dirPath, { recursive: true })
}

/**
 * Read file content safely with error handling
 * @param filePath Path to the file
 * @param encoding File encoding (default: utf-8)
 * @returns File content or null if file doesn't exist
 */
export async function readFileSafe(
  filePath: string,
  encoding: BufferEncoding = 'utf-8',
): Promise<string | null> {
  try {
    return await fs.readFile(filePath, encoding)
  } catch (error: unknown) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
      return null
    }
    throw error
  }
}

/**
 * Write file content safely, creating parent directories if needed
 * @param filePath Path to the file
 * @param content Content to write
 * @param encoding File encoding (default: utf-8)
 */
export async function writeFileSafe(
  filePath: string,
  content: string,
  encoding: BufferEncoding = 'utf-8',
): Promise<void> {
  const path = await import('node:path')
  const dir = path.dirname(filePath)
  await ensureDirectory(dir)
  await fs.writeFile(filePath, content, encoding)
}

/**
 * Get file stats if the file exists
 * @param filePath Path to the file
 * @returns File stats or null if file doesn't exist
 */
export async function getFileStats(
  filePath: string,
): Promise<Awaited<ReturnType<typeof fs.stat>> | null> {
  try {
    return await fs.stat(filePath)
  } catch (error: unknown) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
      return null
    }
    throw error
  }
}

/**
 * Check if a path is a directory
 * @param dirPath Path to check
 * @returns True if the path is a directory, false otherwise
 */
export async function isDirectory(dirPath: string): Promise<boolean> {
  const stats = await getFileStats(dirPath)
  return stats?.isDirectory() ?? false
}

/**
 * Check if a path is a file
 * @param filePath Path to check
 * @returns True if the path is a file, false otherwise
 */
export async function isFile(filePath: string): Promise<boolean> {
  const stats = await getFileStats(filePath)
  return stats?.isFile() ?? false
}
