// Services

// Routes
export { registerAuthRoutes } from './api/auth.routes.js'
export { Auth0OAuthProvider, createAuth0OAuthProvider } from './providers/auth0.provider.js'
// Providers
export { createGitHubOAuthProvider, GitHubOAuthProvider } from './providers/github.provider.js'
export type { OAuthProvider } from './providers/oauth-provider.interface.js'
export { AuthService, createAuthService } from './services/auth.service.js'
export { createSessionService, SessionService } from './services/session.service.js'
export { SessionContext } from './services/session-context.js'

// Storage
export { createSessionStorage, SessionStorage } from './storage/session.storage.js'

// Utils
export {
  getSessionCookieOptions,
  getSessionMaxAge,
  SESSION_COOKIE_NAME,
} from './utils/cookie.utils.js'
