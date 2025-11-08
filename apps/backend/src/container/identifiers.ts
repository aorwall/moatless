import type { ServiceIdentifier } from 'inversify'
import type { PreviewServerLifecycleSink } from '../features/preview/types/index.js'

/**
 * DI symbol for PreviewServerLifecycleSink (from preview feature module).
 * This symbol is kept here since it's used across multiple features.
 *
 * Other DI symbols are colocated with their interface definitions:
 * - InfrastructureStorageSymbol: infrastructure/storage/storage.ts
 * - GitHubClientSymbol: infrastructure/clients/github.client.ts
 * - Auth0ClientSymbol: infrastructure/clients/auth0.client.ts
 * - OAuthProvidersSymbol: features/auth/providers/oauth-provider.interface.ts
 */
export const PreviewServerLifecycleSinkSymbol: ServiceIdentifier<PreviewServerLifecycleSink> =
  Symbol.for('PreviewServerLifecycleSink')
