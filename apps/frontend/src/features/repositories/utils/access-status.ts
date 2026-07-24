/**
 * Repository provider-reachability (`access_status`), stamped when a repo is
 * auto-created by a git-synced workspace. `unchecked`/`ok` are healthy; the
 * rest are surfaced as fail states in the admin UI.
 */
export type RepoAccessFailure = 'not_found' | 'forbidden' | 'error'

/** Whether an `access_status` value indicates a reachability problem. */
export function isAccessFailure(status: string | null | undefined): status is RepoAccessFailure {
  return status === 'not_found' || status === 'forbidden' || status === 'error'
}

/** Short badge label for a failing `access_status`. */
export function accessFailureLabel(status: string | null | undefined): string {
  switch (status) {
    case 'not_found':
      return 'Not found'
    case 'forbidden':
      return 'No access'
    case 'error':
      return 'Unreachable'
    default:
      return 'Access issue'
  }
}

/** Human-readable label for every repository access state. */
export function accessStatusLabel(status: string | null | undefined): string {
  switch (status) {
    case 'ok':
      return 'Verified'
    case 'unchecked':
    case undefined:
    case null:
      return 'Not checked'
    default:
      return accessFailureLabel(status)
  }
}
