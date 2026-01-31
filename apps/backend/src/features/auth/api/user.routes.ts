import type { OpenAPIHono } from '@hono/zod-openapi'
import { listUsersRoute } from '@moatless/api-contract/users'
import type { UserSummary } from '@moatless/types'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { UserRepository } from '../db/user.repository'

export function registerUserRoutes(app: OpenAPIHono<AppBindings>) {
  app.openapi(listUsersRoute, async (c) => {
    const userRepository = appContainer.get(UserRepository)
    const allUsers = await userRepository.getAllUsers()

    const users: UserSummary[] = allUsers.map((user) => ({
      id: user.id,
      login: user.login,
      name: user.name ?? null,
      avatarUrl: user.avatarUrl ?? null,
    }))

    return c.json({ users })
  })
}
