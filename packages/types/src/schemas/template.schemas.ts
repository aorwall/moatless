import { z } from 'zod'
import { PreviewConfigSchema } from './repository.schemas.js'

export const RepositoryTemplateSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  gitUrl: z.string().url('Must be a valid Git URL'),
  setupCommand: z.string().optional(),
  previewConfig: PreviewConfigSchema.optional(),
})

export const TemplateListResponseSchema = z.object({
  templates: z.array(RepositoryTemplateSchema),
})

export const TemplateIdParamsSchema = z.object({
  templateId: z.string(),
})

export type RepositoryTemplate = z.infer<typeof RepositoryTemplateSchema>
export type TemplateListResponse = z.infer<typeof TemplateListResponseSchema>
export type TemplateIdParams = z.infer<typeof TemplateIdParamsSchema>
