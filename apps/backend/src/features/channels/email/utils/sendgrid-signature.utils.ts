import { createHmac, timingSafeEqual } from 'node:crypto'

/**
 * Verify SendGrid webhook signature using HMAC-SHA256.
 *
 * SendGrid sends the signature in the X-Twilio-Email-Event-Webhook-Signature header
 * and the timestamp in X-Twilio-Email-Event-Webhook-Timestamp.
 *
 * @param signature - The signature from X-Twilio-Email-Event-Webhook-Signature header
 * @param timestamp - The timestamp from X-Twilio-Email-Event-Webhook-Timestamp header
 * @param payload - The raw request body
 * @param secret - The webhook signing secret
 * @returns True if signature is valid
 */
export function verifySendGridSignature(
  signature: string | undefined,
  timestamp: string | undefined,
  payload: string,
  secret: string,
): boolean {
  if (!(signature && timestamp && secret)) {
    return false
  }

  // SendGrid signature is: base64(hmac-sha256(timestamp + payload, secret))
  const signedPayload = timestamp + payload
  const expectedSignature = createHmac('sha256', secret).update(signedPayload).digest('base64')

  // Use timing-safe comparison to prevent timing attacks
  try {
    const signatureBuffer = Buffer.from(signature, 'base64')
    const expectedBuffer = Buffer.from(expectedSignature, 'base64')

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
 * Skip verification when no secret is configured.
 *
 * Note: SendGrid Inbound Parse webhook does NOT send signature headers by default.
 * Signature verification is only available for the Event Webhook.
 * If you configure SENDGRID_WEBHOOK_SECRET, verification will be enabled.
 *
 * @param secret - The webhook secret
 * @returns True if verification should be skipped
 */
export function shouldSkipVerification(secret: string | undefined): boolean {
  // Skip verification when no secret is configured
  // Inbound Parse webhooks don't use signatures by default
  return !secret
}
