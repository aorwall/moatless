// Re-export all settings schemas and types

export * from '../schemas/settings/index.js'
// Legacy alias for backwards compatibility
export type { UpdateSettingsSchemaType as UpdateSettingsRequest } from '../schemas/settings.schemas.js'
export * from '../schemas/settings.schemas.js'
