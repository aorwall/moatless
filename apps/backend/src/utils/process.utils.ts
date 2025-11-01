/**
 * Process utility functions for checking process status
 */

/**
 * Check if a process with the given PID is currently running
 * Uses kill(pid, 0) which doesn't actually kill the process but checks if it exists
 *
 * @param pid - Process ID to check
 * @returns true if process exists and is running, false otherwise
 */
export function isProcessAlive(pid: number | undefined): boolean {
  if (!pid) {
    return false
  }

  try {
    // Signal 0 is a special case - it checks if the process exists without sending a signal
    // If the process exists, kill returns without error
    // If the process doesn't exist, it throws ESRCH error
    process.kill(pid, 0)
    return true
  } catch (error) {
    // ESRCH means "No such process"
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ESRCH') {
      return false
    }
    // EPERM means we don't have permission but the process exists
    if (error && typeof error === 'object' && 'code' in error && error.code === 'EPERM') {
      return true
    }
    // Any other error, assume process doesn't exist
    return false
  }
}
