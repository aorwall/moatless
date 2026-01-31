import type { OpenAPIHono } from '@hono/zod-openapi'
import { listTagsRoute } from '@moatless/api-contract/tags'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { TagRepository } from '../db/tag.repository'

export function registerTagRoutes(app: OpenAPIHono<AppBindings>) {
  app.openapi(listTagsRoute, async (c) => {
    const tagRepository = appContainer.get(TagRepository)
    const tags = await tagRepository.listAll()
    return c.json({ tags })
  })
}
