// Re-export types from schemas
export type {
  Command,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '../schemas/command.schemas.js'

import type { Command } from '../schemas/command.schemas.js'

// Additional types not in schemas
export interface CommandsFile {
  commands: Record<string, Command>
}

export interface CommandList {
  commands: Command[]
}
