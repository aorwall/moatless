import type { OpenAPIHono } from '@hono/zod-openapi'
import { listTemplatesRoute } from '@moatless/api-contract/templates'
import type { TemplateListResponse } from '@moatless/types'
import type { AppBindings } from '../app'
import { TEMPLATES } from '../config/templates'

export function registerTemplateRoutes(app: OpenAPIHono<AppBindings>) {
  // List templates
  app.openapi(listTemplatesRoute, (c) => {
    // Serialize templates without systemPrompt (internal only)
    const templates = TEMPLATES.map((t) => ({
      id: t.id,
      name: t.name,
      gitUrl: t.gitUrl,
      setupCommand: t.setupCommand,
      previewConfig: t.previewConfig,
    }))

    return c.json({ templates } as TemplateListResponse)
  })
}
