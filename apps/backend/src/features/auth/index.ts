// Services

// Routes
export { registerAuthRoutes } from './api/auth.routes'
export { Auth0OAuthProvider, createAuth0OAuthProvider } from './providers/auth0.provider'
// Providers
export { createGitHubOAuthProvider, GitHubOAuthProvider } from './providers/github.provider'
export type { OAuthProvider } from './providers/oauth-provider.interface'
export { AuthService } from './services/auth.service'
export { SessionService } from './services/session.service'
export { SessionContext } from './services/session-context'

// Storage
export { SessionStorage } from './storage/session.storage'
export { UserStorage } from './storage/user.storage'

// Utils
export {
  getSessionCookieOptions,
  getSessionMaxAge,
  SESSION_COOKIE_NAME,
} from './utils/cookie.utils'
