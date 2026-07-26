import type { ServerConfig } from '@moatless/api-client/sandbox/model'

/**
 * Run-configuration fields shared by repositories and workspaces.
 *
 * Both `UpdateRepositoryRequest` and `UpdateWorkspaceRequest` (and their
 * `Create*` counterparts) carry this exact camelCase subset with identical
 * value types, so the section-group components below are agnostic to which
 * config source is being edited. A repository/workspace form's all-optional
 * request type is assignable to this shape (extra fields are ignored), and its
 * `updateField` setter is assignable to {@link UpdateRunConfigField}.
 */
export interface RunConfigFormValues {
  systemPrompt?: string | null
  dockerImage?: string | null
  setupCommands?: string[] | null
  envVars?: { [key: string]: string } | null
  servers?: ServerConfig[] | null
}

/** Partial-update setter for a single {@link RunConfigFormValues} field. */
export type UpdateRunConfigField = <K extends keyof RunConfigFormValues>(
  field: K,
  value: RunConfigFormValues[K],
) => void
