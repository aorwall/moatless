// Routes
export { registerAuthRoutes } from './api/auth.routes'
export { SessionRepository } from './db/session.repository'
export type { SessionInsert, SessionSelect } from './db/session.schema'
export { sessions as sessionsSchema } from './db/session.schema'
// Database repositories (Drizzle ORM)
export { UserRepository } from './db/user.repository'
export type { UserInsert, UserSelect } from './db/user.schema'
export { users as usersSchema } from './db/user.schema'

// Providers
export { Auth0OAuthProvider, createAuth0OAuthProvider } from './providers/auth0.provider'
export { createGitHubOAuthProvider, GitHubOAuthProvider } from './providers/github.provider'
export type { OAuthProvider } from './providers/oauth-provider.interface'

// Services
export { AuthService } from './services/auth.service'
export { SessionService } from './services/session.service'
export { SessionContext } from './services/session-context'

// Utils
export {
  getSessionCookieOptions,
  getSessionMaxAge,
  SESSION_COOKIE_NAME,
} from './utils/cookie.utils'
