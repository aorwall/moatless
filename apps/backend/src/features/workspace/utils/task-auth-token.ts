import * as jose from 'jose'

/**
 * Generate a task-scoped JWT token for sandbox-to-backend MCP communication.
 *
 * The token is signed with TASK_TOKEN_SECRET (HS256) and expires after 24 hours.
 * Both Kubernetes and Docker provisioners use this to authenticate sandbox pods/containers.
 */
export async function generateTaskAuthToken(taskId: string, userId: string): Promise<string> {
  const secret = process.env['TASK_TOKEN_SECRET']
  if (!secret) {
    throw new Error('TASK_TOKEN_SECRET not configured')
  }

  const secretKey = new TextEncoder().encode(secret)
  const token = await new jose.SignJWT({ taskId, userId })
    .setProtectedHeader({ alg: 'HS256' })
    .setExpirationTime('24h')
    .setIssuedAt()
    .sign(secretKey)

  return token
}
