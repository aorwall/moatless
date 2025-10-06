import { exec, spawn } from 'node:child_process'

const isWindows = process.platform === 'win32'

type Signal = NodeJS.Signals | number
type Callback = (error?: Error) => void

interface ProcessTree {
  [pid: string]: string[]
}

interface PidsToProcess {
  [pid: string]: number
}

/**
 * Kill a process and all its children
 * @param pid Process ID to kill
 * @param signal Signal to send (default: SIGTERM)
 * @param callback Optional callback function
 */
export function treeKill(pid: number, signal?: Signal | Callback, callback?: Callback): void {
  // Handle optional signal parameter
  let actualSignal: Signal = 'SIGTERM'
  let actualCallback: Callback | undefined = callback

  if (typeof signal === 'function') {
    actualCallback = signal
    actualSignal = 'SIGTERM'
  } else if (signal) {
    actualSignal = signal
  }

  if (isWindows) {
    // On Windows, use taskkill with /T flag to kill process tree
    exec(`taskkill /pid ${pid} /T /F`, (error) => {
      if (actualCallback) {
        actualCallback(error || undefined)
      }
    })
  } else {
    // On Unix systems, build process tree and kill all processes
    const tree: ProcessTree = {}
    tree[pid.toString()] = []
    const pidsToProcess: PidsToProcess = {}
    pidsToProcess[pid.toString()] = 1

    buildProcessTree(pid, tree, pidsToProcess, (error) => {
      if (error && actualCallback) {
        return actualCallback(error)
      }

      try {
        killAll(tree, actualSignal)
      } catch (err) {
        if (actualCallback) {
          return actualCallback(err as Error)
        }
        throw err
      }

      if (actualCallback) {
        actualCallback()
      }
    })
  }
}

/**
 * Promise-based version of treeKill for easier async/await usage
 */
export function treeKillAsync(pid: number, signal: Signal = 'SIGTERM'): Promise<void> {
  return new Promise((resolve, reject) => {
    treeKill(pid, signal, (error) => {
      if (error) {
        reject(error)
      } else {
        resolve()
      }
    })
  })
}

function killAll(tree: ProcessTree, signal: Signal): void {
  const killed: { [pid: string]: number } = {}

  // Kill children first, then parents
  for (const pid of Object.keys(tree)) {
    const children = tree[pid]
    if (!children) {
      continue
    }
    for (const childPid of children) {
      if (!killed[childPid]) {
        killPid(Number.parseInt(childPid, 10), signal)
        killed[childPid] = 1
      }
    }
    if (!killed[pid]) {
      killPid(Number.parseInt(pid, 10), signal)
      killed[pid] = 1
    }
  }
}

function killPid(pid: number, signal: Signal): void {
  try {
    process.kill(pid, signal)
  } catch (err) {
    const error = err as NodeJS.ErrnoException
    // ESRCH means 'No such process' - this is acceptable
    if (error.code !== 'ESRCH') {
      throw err
    }
  }
}

function buildProcessTree(
  _parentPid: number,
  tree: ProcessTree,
  pidsToProcess: PidsToProcess,
  callback: Callback,
): void {
  const ps = spawn('ps', ['-o', 'pid,ppid'])
  let allData = ''

  ps.stdout.on('data', (data: Buffer) => {
    allData += data.toString('utf8')
  })

  ps.on('close', (code: number | null) => {
    if (code !== 0) {
      // ps command failed
      if (code === null) {
        return callback(new Error('ps command was killed'))
      }
      return callback(new Error(`ps command exited with code ${code}`))
    }

    // Parse ps output and build tree iteratively
    const lines = allData.split('\n')
    let foundNewChildren = true

    // Keep iterating until we don't find any new children
    while (foundNewChildren) {
      foundNewChildren = false

      for (let i = 1; i < lines.length; i++) {
        const currentLine = lines[i]
        if (!currentLine) {
          continue
        }
        const line = currentLine.trim()
        if (!line) {
          continue
        }

        const parts = line.split(/\s+/)
        if (parts.length < 2) {
          continue
        }

        const pid = parts[0]
        const ppid = parts[1]

        if (!(pid && ppid)) {
          continue
        }

        // If this process's parent is in our list, and we haven't seen this PID yet
        if (pidsToProcess[ppid] && !pidsToProcess[pid]) {
          if (!tree[ppid]) {
            tree[ppid] = []
          }
          tree[ppid].push(pid)
          pidsToProcess[pid] = 1
          foundNewChildren = true
        }
      }
    }

    callback()
  })

  ps.on('error', (error: Error) => {
    callback(error)
  })
}
