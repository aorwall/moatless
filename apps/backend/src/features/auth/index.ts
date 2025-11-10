// Routes
export { registerAuthRoutes } from './api/auth.routes'
export { registerGitHubSettingsRoutes } from './api/github-settings.routes'
export { registerGitLabSettingsRoutes } from './api/gitlab-settings.routes'

// Database repositories (Drizzle ORM)
export { OAuthStateRepository } from './db/oauth-state.repository'
export type { OAuthState, OAuthStateInsert } from './db/oauth-state.schema'
export { oauthStates as oauthStatesSchema } from './db/oauth-state.schema'
export { ProviderTokenRepository } from './db/provider-token.repository'
export type { ProviderTokenInsert, ProviderTokenSelect } from './db/provider-token.schema'
export { providerTokens as providerTokensSchema } from './db/provider-token.schema'
export { SessionRepository } from './db/session.repository'
export type { SessionInsert, SessionSelect } from './db/session.schema'
export { sessions as sessionsSchema } from './db/session.schema'
export { UserRepository } from './db/user.repository'
export type { UserInsert, UserSelect } from './db/user.schema'
export { users as usersSchema } from './db/user.schema'

// Providers
export { Auth0OAuthProvider, createAuth0OAuthProvider } from './providers/auth0.provider'
export { createGitHubOAuthProvider, GitHubOAuthProvider } from './providers/github.provider'
export type { OAuthProvider } from './providers/oauth-provider.interface'

// Services
export { AuthService } from './services/auth.service'
export { GitHubTokenService } from './services/github-token.service'
export { SessionService } from './services/session.service'
export { SessionContext } from './services/session-context'

// Utils
export {
  getSessionCookieOptions,
  getSessionMaxAge,
  SESSION_COOKIE_NAME,
} from './utils/cookie.utils'
