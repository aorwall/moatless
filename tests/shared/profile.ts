/**
 * Environment profiles for the E2E suite.
 *
 * One suite runs against three environments: a local Docker Compose stack, the
 * deployed Kubernetes environment, and a backend `scripts/local-e2e/run.sh`
 * starts inside a sandbox. Everything that differs between them lives here, so a
 * test never branches on which environment it is in — it asks the profile for
 * a property, or guards on a capability.
 *
 * Selection is `E2E_PROFILE`. When unset, a process that already carries a
 * sandbox login is a cluster run, and anything else is a Docker run. The
 * resolved name is always recorded and printed, never left implicit: a
 * misconfigured run must not silently test the wrong environment.
 */

export type ProfileName = 'docker' | 'k8s' | 'local'

/**
 * What the active environment can do. A test that needs a capability the
 * profile lacks skips there — visibly, with the capability named — rather than
 * failing or being deleted.
 */
export interface Capabilities {
  /** Create additional users and log in as them. Local password auth only. */
  seedUsers: boolean
  /** Truncate the database between runs. */
  resetDatabase: boolean
  /** Read or write the database directly, for state the API does not expose. */
  directDatabase: boolean
  /** The backend can reach an HTTP server hosted by the test process. */
  inboundCallbacks: boolean
  /** Claude Code credentials are configured, so Claude agent tests can run. */
  claudeCredentials: boolean
  /** The backend holds a Codex connection, so a Codex task can be created. */
  codexConnection: boolean
  /** The run can create and delete real repositories on GitHub — it has a PAT. */
  githubRepositories: boolean
  /** The runtime schedules onto this machine, so an image built here can run. */
  localImages: boolean
  /**
   * A stopped task's files can be read from the store. Needs an S3 bucket: with
   * none, a stopped sandbox has no durable copy and every read answers "not
   * running".
   */
  storedWorkspace: boolean
  /** A runtime watcher publishes `sandbox.ready` when a sandbox comes up. */
  sandboxReadyEvent: boolean
  /** The runtime runs the workspace's `dockerImage`, so a bad image fails the sandbox. */
  workspaceImage: boolean
  /**
   * The backend runs with `FF_TRAJECTORY_RECORDING`, and with either
   * `FF_TRAJECTORY_RESUME` or the fixture workspace setting
   * `MOATLESS_RESUME_FROM_TRAJECTORY=1`. None is visible through the API, so
   * the operator says so with `E2E_TRAJECTORY=1`.
   */
  trajectory: boolean
}

/** How long to wait for the slow, environment-dependent transitions. */
export interface Timeouts {
  /** Sandbox reaches ready. */
  sandboxReady: number
  /** Sandbox reaches stopped, or is gone entirely. */
  sandboxTeardown: number
  /** Agent returns to idle after a turn. */
  agentIdle: number
}

/** Where tests get a repository to run tasks against. */
export interface Fixture {
  /** Pre-existing repository to use, when the environment has one. */
  repositoryId?: string
  /** Pre-existing workspace to use, when the environment has one. */
  workspaceId?: string
  /** Template to create a repository from, when it does not. */
  templateUrl?: string
}

export interface Profile {
  name: ProfileName
  baseUrl: string
  /** `password` mints an API key by logging in; `ambient` uses the sandbox's own login. */
  auth: 'password' | 'ambient'
  fixture: Fixture
  timeouts: Timeouts
  capabilities: Capabilities
}

const DEFAULT_TEMPLATE_URL = 'https://github.com/aorwall/hello-world-ui.git'

/** Moatable — the deployed environment's React + Vite starter template. */
const MOATABLE_REPOSITORY_ID = '433e035b-f37d-4992-834a-01ed779996e7'
const MOATABLE_WORKSPACE_ID = 'ws_433e035b-f37d-4992-834a-01ed779996e7'

function dockerProfile(): Profile {
  return {
    name: 'docker',
    baseUrl:
      process.env.DOCKER_COMPOSE_URL || `http://localhost:${process.env.BACKEND_PORT || '8080'}`,
    auth: 'password',
    fixture: { templateUrl: process.env.E2E_TEMPLATE_URL || DEFAULT_TEMPLATE_URL },
    timeouts: {
      sandboxReady: 120_000,
      sandboxTeardown: 60_000,
      // A turn against a real model is the one thing Docker does not make
      // faster, so this matches the cluster more closely than the rest.
      agentIdle: 300_000,
    },
    capabilities: {
      seedUsers: true,
      resetDatabase: true,
      directDatabase: true,
      inboundCallbacks: true,
      claudeCredentials: hasClaudeCredentials(),
      // The compose stack seeds no Codex connection.
      codexConnection: false,
      githubRepositories: Boolean(process.env.GITHUB_PAT),
      localImages: true,
      // The compose stack configures no bucket, so nothing is stored.
      storedWorkspace: false,
      sandboxReadyEvent: true,
      workspaceImage: true,
      trajectory: process.env.E2E_TRAJECTORY === '1',
    },
  }
}

function k8sProfile(): Profile {
  return {
    name: 'k8s',
    baseUrl:
      process.env.MOATLESS_API_URL ||
      process.env.BACKEND_API_URL ||
      'http://moatless-vibe-backend.moatless.svc.cluster.local:8080',
    auth: 'ambient',
    fixture: {
      repositoryId: process.env.E2E_REPOSITORY_ID || MOATABLE_REPOSITORY_ID,
      workspaceId: process.env.E2E_WORKSPACE_ID || MOATABLE_WORKSPACE_ID,
    },
    // Pods schedule, pull and mount volumes before anything starts, so every
    // ceiling here is well above its Docker equivalent.
    timeouts: {
      sandboxReady: 420_000,
      sandboxTeardown: 120_000,
      agentIdle: 420_000,
    },
    capabilities: {
      // Human users cannot be created under the deployed auth provider, the
      // database is shared and long-lived, there is no database access from a
      // sandbox, and the backend cannot route to a port the runner opens.
      seedUsers: false,
      resetDatabase: false,
      directDatabase: false,
      inboundCallbacks: false,
      claudeCredentials: true,
      codexConnection: true,
      // A sandbox carries no PAT of its own, and the cluster pulls images
      // from a registry — a local `docker build` is not reachable from it.
      githubRepositories: Boolean(process.env.GITHUB_PAT),
      localImages: false,
      storedWorkspace: true,
      sandboxReadyEvent: true,
      workspaceImage: true,
      trajectory: process.env.E2E_TRAJECTORY === '1',
    },
  }
}

/**
 * A backend started by `scripts/local-e2e/run.sh`: this checkout's build, its
 * own Postgres schema, and each sandbox a child process on this machine.
 */
function localProfile(): Profile {
  return {
    name: 'local',
    baseUrl: process.env.MOATLESS_API_URL || 'http://127.0.0.1:8080',
    auth: 'password',
    fixture: { templateUrl: process.env.E2E_TEMPLATE_URL || DEFAULT_TEMPLATE_URL },
    // A process starts in seconds; only the agent's turn is as slow as anywhere.
    timeouts: {
      sandboxReady: 120_000,
      sandboxTeardown: 60_000,
      agentIdle: 300_000,
    },
    capabilities: {
      seedUsers: true,
      // The schema is new and dropped after the run, so there is nothing to reset.
      resetDatabase: false,
      // The database helpers reach Postgres through `docker exec`.
      directDatabase: false,
      inboundCallbacks: true,
      claudeCredentials: hasClaudeCredentials(),
      // The harness seeds one when `OPENAI_API_KEY` is set.
      codexConnection: Boolean(process.env.OPENAI_API_KEY),
      githubRepositories: Boolean(process.env.GITHUB_PAT),
      // Sandboxes are processes, not images.
      localImages: false,
      storedWorkspace: false,
      // `sandbox-local` has no lifecycle watcher.
      sandboxReadyEvent: false,
      // Sandboxes run on the host; `dockerImage` is ignored.
      workspaceImage: false,
      trajectory: process.env.E2E_TRAJECTORY === '1',
    },
  }
}

function hasClaudeCredentials(): boolean {
  return Boolean(process.env.CLAUDE_CODE_OAUTH_TOKEN || process.env.ANTHROPIC_API_KEY)
}

/** The profile name, from `E2E_PROFILE` or inferred from the environment. */
export function resolveProfileName(): ProfileName {
  const requested = process.env.E2E_PROFILE?.trim()
  if (requested) {
    if (requested !== 'docker' && requested !== 'k8s' && requested !== 'local') {
      throw new Error(`Unknown E2E_PROFILE '${requested}'. Valid profiles: docker, k8s, local.`)
    }
    return requested
  }
  // A sandbox login is only present inside a sandbox, which only exists in the
  // deployed environment.
  return process.env.USER_AUTH_TOKEN ? 'k8s' : 'docker'
}

let cached: Profile | null = null

/** The active profile, resolved once per process. */
export function profile(): Profile {
  if (!cached) {
    const builders = { docker: dockerProfile, k8s: k8sProfile, local: localProfile }
    cached = builders[resolveProfileName()]()
  }
  return cached
}

/** Room for the calls a test makes around the transitions it waits on. */
const OVERHEAD_MS = 60_000

/**
 * A Playwright budget for a test, from the transitions it waits on.
 *
 * A test states what it is waiting for rather than a number, so the cluster's
 * far higher ceilings apply automatically. The allowance on top covers the CLI
 * round trips and assertions around the waits — it is not a wait of its own.
 */
export function budget(...waits: (keyof Timeouts)[]): number {
  const t = profile().timeouts
  return waits.reduce((total, key) => total + t[key], OVERHEAD_MS)
}

/** Reset the memoized profile. Tests of the profile itself use this. */
export function resetProfileCache(): void {
  cached = null
}

/** One line naming the environment under test, for the report header. */
export function describeProfile(p: Profile = profile()): string {
  const off = (Object.keys(p.capabilities) as (keyof Capabilities)[]).filter(
    (key) => !p.capabilities[key],
  )
  const missing = off.length > 0 ? `without ${off.join(', ')}` : 'all capabilities'
  return `profile '${p.name}' → ${p.baseUrl} (${missing})`
}

/**
 * Skip reason for a test the active profile cannot run, or `null` when it can.
 *
 * Returning a reason rather than skipping directly keeps the decision in the
 * test file, where the reader can see which capability is missing.
 */
export function missingCapability(...required: (keyof Capabilities)[]): string | null {
  const p = profile()
  const absent = required.filter((key) => !p.capabilities[key])
  if (absent.length === 0) {
    return null
  }
  return `profile '${p.name}' has no ${absent.join(', ')}`
}
