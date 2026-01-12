import { execSync } from 'node:child_process'
import os from 'node:os'

export async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { timeout: 5000, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

export function isPlatformCompatible(): boolean {
  const arch = os.arch()
  return arch === 'x64' || arch === 'arm64'
}
