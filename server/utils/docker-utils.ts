import { exec } from 'child_process'
import { promisify } from 'util'

const execAsync = promisify(exec)

export interface DockerContainer {
  id: string
  name: string
  image: string
  status: string
  created: Date
}

export interface DockerImage {
  id: string
  repository: string
  tag: string
  size: string
  created: Date
}

export async function listContainers(all: boolean = false): Promise<DockerContainer[]> {
  try {
    const command = all ? 'docker ps -a --format json' : 'docker ps --format json'
    const { stdout } = await execAsync(command, { timeout: 10000 })

    if (!stdout.trim()) return []

    const lines = stdout.trim().split('\n')
    return lines.map((line) => {
      const container = JSON.parse(line)
      return {
        id: container.ID,
        name: container.Names,
        image: container.Image,
        status: container.Status,
        created: new Date(container.CreatedAt),
      }
    })
  } catch (error) {
    throw new Error(
      `Failed to list containers: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function containerExists(nameOrId: string): Promise<boolean> {
  try {
    await execAsync(`docker inspect "${nameOrId}"`, { timeout: 5000 })
    return true
  } catch {
    return false
  }
}

export async function isContainerRunning(nameOrId: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync(`docker inspect -f '{{.State.Running}}' "${nameOrId}"`, {
      timeout: 5000,
    })
    return stdout.trim() === 'true'
  } catch {
    return false
  }
}

export async function removeContainer(nameOrId: string, force: boolean = false): Promise<void> {
  try {
    const command = force ? `docker rm -f "${nameOrId}"` : `docker rm "${nameOrId}"`
    await execAsync(command, { timeout: 10000 })
  } catch (error) {
    throw new Error(
      `Failed to remove container: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function executeInContainer(
  nameOrId: string,
  command: string,
  options: {
    workdir?: string
    user?: string
    env?: Record<string, string>
    timeout?: number
  } = {},
): Promise<{ stdout: string; stderr: string }> {
  try {
    let dockerCommand = `docker exec`

    if (options.workdir) {
      dockerCommand += ` -w "${options.workdir}"`
    }

    if (options.user) {
      dockerCommand += ` -u "${options.user}"`
    }

    if (options.env) {
      for (const [key, value] of Object.entries(options.env)) {
        dockerCommand += ` -e "${key}=${value}"`
      }
    }

    dockerCommand += ` "${nameOrId}" ${command}`

    const { stdout, stderr } = await execAsync(dockerCommand, {
      timeout: options.timeout || 30000,
    })

    return { stdout, stderr }
  } catch (error) {
    throw new Error(
      `Failed to execute in container: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function copyToContainer(
  nameOrId: string,
  hostPath: string,
  containerPath: string,
): Promise<void> {
  try {
    await execAsync(`docker cp "${hostPath}" "${nameOrId}:${containerPath}"`, {
      timeout: 30000,
    })
  } catch (error) {
    throw new Error(
      `Failed to copy to container: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function copyFromContainer(
  nameOrId: string,
  containerPath: string,
  hostPath: string,
): Promise<void> {
  try {
    await execAsync(`docker cp "${nameOrId}:${containerPath}" "${hostPath}"`, {
      timeout: 30000,
    })
  } catch (error) {
    throw new Error(
      `Failed to copy from container: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getContainerLogs(
  nameOrId: string,
  options: {
    tail?: number
    since?: string
    timestamps?: boolean
  } = {},
): Promise<string> {
  try {
    let command = `docker logs "${nameOrId}"`

    if (options.tail) {
      command += ` --tail ${options.tail}`
    }

    if (options.since) {
      command += ` --since "${options.since}"`
    }

    if (options.timestamps) {
      command += ' --timestamps'
    }

    const { stdout } = await execAsync(command, { timeout: 10000 })
    return stdout
  } catch (error) {
    throw new Error(
      `Failed to get container logs: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}
