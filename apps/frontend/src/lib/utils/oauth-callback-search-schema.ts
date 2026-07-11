import { z } from 'zod'

/**
 * Search params delivered by OAuth providers to our callback routes.
 * `.catch(undefined)` tolerates malformed values (e.g. `?code[]=x`) instead
 * of throwing during route matching — the components already handle absence.
 */
export const oauthCallbackSearchSchema = z.object({
  code: z.string().optional().catch(undefined),
  state: z.string().optional().catch(undefined),
})
