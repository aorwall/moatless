// Re-export types from schemas
export type {
  Command,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '../schemas/command.schemas.js'

import type { Command } from '../schemas/index.js'

// Additional types not in schemas

export interface CommandList {
  commands: Command[]
}
