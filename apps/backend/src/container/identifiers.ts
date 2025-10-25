import type { PreviewServerLifecycleSink } from '@moatless/preview-server'
import type { ServiceIdentifier } from 'inversify'

/**
 * DI symbol for PreviewServerLifecycleSink (from external @moatless/preview-server package).
 * This symbol is kept here since it's for an external package interface.
 *
 * Other DI symbols are colocated with their interface definitions:
 * - StorageSymbol: storage/storage-backend.ts
 * - GitHubClientSymbol: infrastructure/clients/github.client.ts
 * - Auth0ClientSymbol: infrastructure/clients/auth0.client.ts
 * - OAuthProvidersSymbol: features/auth/providers/oauth-provider.interface.ts
 */
export const PreviewServerLifecycleSinkSymbol: ServiceIdentifier<PreviewServerLifecycleSink> =
  Symbol.for('PreviewServerLifecycleSink')
