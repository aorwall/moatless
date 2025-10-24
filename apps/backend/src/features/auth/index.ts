// Services

// Routes
export { registerAuthRoutes } from './api/auth.routes.js'
export { Auth0OAuthProvider, createAuth0OAuthProvider } from './providers/auth0.provider.js'
// Providers
export { createGitHubOAuthProvider, GitHubOAuthProvider } from './providers/github.provider.js'
export type { OAuthProvider } from './providers/oauth-provider.interface.js'
export { AuthService } from './services/auth.service.js'
export { SessionService } from './services/session.service.js'
export { SessionContext } from './services/session-context.js'

// Storage
export { SessionStorage } from './storage/session.storage.js'
export { UserStorage } from './storage/user.storage.js'

// Utils
export {
  getSessionCookieOptions,
  getSessionMaxAge,
  SESSION_COOKIE_NAME,
} from './utils/cookie.utils.js'
