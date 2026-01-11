import { createHmac, timingSafeEqual } from 'node:crypto'

/**
 * Verify Slack request signature using HMAC-SHA256.
 *
 * Slack sends the signature in X-Slack-Signature header
 * and the timestamp in X-Slack-Request-Timestamp.
 *
 * @param signature - The signature from X-Slack-Signature header (v0=...)
 * @param timestamp - The timestamp from X-Slack-Request-Timestamp header
 * @param body - The raw request body
 * @param signingSecret - The Slack signing secret
 * @returns True if signature is valid
 */
export function verifySlackSignature(
  signature: string | undefined,
  timestamp: string | undefined,
  body: string,
  signingSecret: string,
): boolean {
  if (!(signature && timestamp && signingSecret)) {
    return false
  }

  // Check timestamp is within 5 minutes to prevent replay attacks
  const now = Math.floor(Date.now() / 1000)
  const requestTime = Number.parseInt(timestamp, 10)
  if (Math.abs(now - requestTime) > 300) {
    return false
  }

  // Compute expected signature: v0=hmac-sha256('v0:{timestamp}:{body}', secret)
  const sigBaseString = `v0:${timestamp}:${body}`
  const expectedSignature = `v0=${createHmac('sha256', signingSecret).update(sigBaseString).digest('hex')}`

  // Use timing-safe comparison to prevent timing attacks
  try {
    const signatureBuffer = Buffer.from(signature)
    const expectedBuffer = Buffer.from(expectedSignature)

    if (signatureBuffer.length !== expectedBuffer.length) {
      return false
    }

    return timingSafeEqual(signatureBuffer, expectedBuffer)
  } catch {
    return false
  }
}

/**
 * Check if webhook verification should be skipped.
 * Skip verification when no signing secret is configured.
 *
 * Note: For production use, you should configure SLACK_SIGNING_SECRET
 * to verify requests are actually coming from Slack.
 *
 * @param signingSecret - The Slack signing secret
 * @returns True if verification should be skipped
 */
export function shouldSkipSlackVerification(signingSecret: string | undefined): boolean {
  // Skip verification when no secret is configured
  return !signingSecret
}
