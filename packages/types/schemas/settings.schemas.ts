import { z } from 'zod'

export const updateSettingsSchema = z
  .object({
    dataDir: z.string().nullable().optional(),
    defaultWorkspaceType: z.enum(['docker', 'local']).nullable().optional(),
    claude: z
      .object({
        binDir: z.string().nullable().optional(),
        configDir: z.string().nullable().optional(),
      })
      .strict()
      .optional(),
  })
  .strict()

export type UpdateSettingsSchemaType = z.infer<typeof updateSettingsSchema>
