import { injectable } from 'inversify'
import { jwtVerify, SignJWT } from 'jose'
import { logger } from '../../../utils/logger'

/**
 * Payload embedded in worker JWT tokens
 */
export interface WorkerTokenPayload {
  userId: string
  taskId: string
  userLogin?: string | undefined
  userEmail?: string | undefined
  groupId?: string | undefined
}

/**
 * Service for generating and verifying JWT tokens for worker pod authentication.
 *
 * Worker pods use these tokens to authenticate back to the main API when emitting events.
 * The token contains the user's identity so that access control works properly.
 */
@injectable()
export class WorkerTokenService {
  private secret: Uint8Array | null = null

  /**
   * Get the signing secret, lazy-loaded to allow for optional K8s mode
   */
  private getSecret(): Uint8Array {
    if (this.secret) {
      return this.secret
    }

    // Use WORKER_JWT_SECRET if set, otherwise fall back to WORKER_API_KEY
    const secretKey = process.env['WORKER_JWT_SECRET'] || process.env['WORKER_API_KEY']
    if (!secretKey) {
      throw new Error('WORKER_JWT_SECRET or WORKER_API_KEY required for worker token generation')
    }

    this.secret = new TextEncoder().encode(secretKey)
    return this.secret
  }

  /**
   * Generate a JWT token for a worker pod.
   *
   * The token is valid for 24 hours and contains the user's identity.
   *
   * @param payload - User and task information to embed in the token
   * @returns Signed JWT token
   */
  async generateToken(payload: WorkerTokenPayload): Promise<string> {
    const token = await new SignJWT({
      ...payload,
      type: 'worker',
    })
      .setProtectedHeader({ alg: 'HS256' })
      .setIssuedAt()
      .setExpirationTime('24h')
      .setSubject(payload.userId)
      .sign(this.getSecret())

    logger.debug(
      { taskId: payload.taskId, userId: payload.userId },
      'WorkerTokenService: Generated worker auth token',
    )

    return token
  }

  /**
   * Verify a JWT token and extract the payload.
   *
   * @param token - JWT token to verify
   * @returns Decoded payload if valid, null if invalid or expired
   */
  async verifyToken(token: string): Promise<WorkerTokenPayload | null> {
    try {
      const { payload } = await jwtVerify(token, this.getSecret())

      // Validate required fields
      if (!(payload.sub && payload['taskId'])) {
        logger.warn('WorkerTokenService: Token missing required fields')
        return null
      }

      // Check token type
      if (payload['type'] !== 'worker') {
        logger.warn({ type: payload['type'] }, 'WorkerTokenService: Invalid token type')
        return null
      }

      const result: WorkerTokenPayload = {
        userId: payload.sub as string,
        taskId: payload['taskId'] as string,
      }

      // Add optional fields only if they exist
      if (payload['userLogin']) {
        result.userLogin = payload['userLogin'] as string
      }
      if (payload['userEmail']) {
        result.userEmail = payload['userEmail'] as string
      }
      if (payload['groupId']) {
        result.groupId = payload['groupId'] as string
      }

      return result
    } catch (error) {
      logger.warn({ err: error as Error }, 'WorkerTokenService: Failed to verify worker auth token')
      return null
    }
  }

  /**
   * Check if worker token service is available (secrets are configured)
   */
  isAvailable(): boolean {
    return !!(process.env['WORKER_JWT_SECRET'] || process.env['WORKER_API_KEY'])
  }
}
