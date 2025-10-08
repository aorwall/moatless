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

    buildProcessTreeFromPs(pid, tree, pidsToProcess, (error) => {
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

function checkPsExitCode(code: number | null): Error | null {
  if (code === 0) {
    return null
  }
  if (code === null) {
    return new Error('ps command was killed')
  }
  return new Error(`ps command exited with code ${code}`)
}

function parseProcessLine(line: string): { pid: string; ppid: string } | null {
  const trimmed = line.trim()
  if (!trimmed) {
    return null
  }

  const parts = trimmed.split(/\s+/)
  if (parts.length < 2) {
    return null
  }

  const pid = parts[0]
  const ppid = parts[1]

  if (!(pid && ppid)) {
    return null
  }

  return { pid, ppid }
}

function addChildProcess(
  pid: string,
  ppid: string,
  pidsToProcess: PidsToProcess,
  tree: ProcessTree,
): boolean {
  // If this process's parent is in our list, and we haven't seen this PID yet
  if (pidsToProcess[ppid] && !pidsToProcess[pid]) {
    if (!tree[ppid]) {
      tree[ppid] = []
    }
    tree[ppid].push(pid)
    pidsToProcess[pid] = 1
    return true
  }
  return false
}

function buildProcessTree(psOutput: string, pidsToProcess: PidsToProcess, tree: ProcessTree): void {
  const lines = psOutput.split('\n')
  let foundNewChildren = true

  // Keep iterating until we don't find any new children
  while (foundNewChildren) {
    foundNewChildren = false

    for (let i = 1; i < lines.length; i++) {
      const currentLine = lines[i]
      if (!currentLine) {
        continue
      }

      const parsed = parseProcessLine(currentLine)
      if (!parsed) {
        continue
      }

      const { pid, ppid } = parsed
      if (addChildProcess(pid, ppid, pidsToProcess, tree)) {
        foundNewChildren = true
      }
    }
  }
}

function buildProcessTreeFromPs(
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
    const error = checkPsExitCode(code)
    if (error) {
      return callback(error)
    }

    try {
      buildProcessTree(allData, pidsToProcess, tree)
      callback()
    } catch (error) {
      callback(error instanceof Error ? error : new Error(String(error)))
    }
  })

  ps.on('error', (error: Error) => {
    callback(error)
  })
}
