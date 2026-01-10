import type { Hono } from 'hono'
import { simpleParser } from 'mailparser'
import type { AppBindings } from '../../../../app'
import { appContainer } from '../../../../container/container'
import { logger } from '../../../../utils/logger'
import { EmailInboundService, type InboundEmail } from '../services/email-inbound.service'

/**
 * Register email webhook routes.
 * These routes handle inbound emails from SendGrid.
 */
export function registerEmailWebhookRoutes(app: Hono<AppBindings>) {
  /**
   * POST /api/v1/channels/email/inbound
   *
   * Receives inbound emails from SendGrid Inbound Parse webhook.
   * Supports both "Raw" mode (full MIME email) and "Parsed" mode (form fields).
   *
   * Note: SendGrid Inbound Parse does NOT use signature verification.
   * Security is handled by:
   * 1. Using a unique/secret email address pattern
   * 2. Validating the recipient matches expected format
   */
  app.post('/api/v1/channels/email/inbound', async (c) => {
    try {
      const formData = await c.req.formData()

      // Try raw MIME mode first (preferred - more reliable parsing)
      const rawEmail = formData.get('email') as string | null
      let email: InboundEmail

      if (rawEmail) {
        email = await parseRawEmail(rawEmail)
      } else {
        // Fall back to parsed mode for backward compatibility
        logger.debug('No raw email in request, falling back to parsed mode')
        email = parseFormDataToEmail(formData)
      }

      if (!email.to) {
        logger.warn('Missing recipient address in inbound email')
        return c.json({ error: 'Missing recipient address' }, 400)
      }

      const emailService = appContainer.get(EmailInboundService)
      const result = await emailService.processEmail(email)

      if (!result.success) {
        logger.warn({ error: result.error }, 'Failed to process inbound email')
        return c.json({ status: 'error', error: result.error })
      }

      return c.json({
        status: 'success',
        taskId: result.taskId,
        isNewTask: result.isNewTask,
      })
    } catch (error) {
      logger.error({ err: error }, 'Error processing inbound email')
      return c.json({ status: 'error', error: 'Internal error' })
    }
  })
}

/**
 * Parse raw MIME email using mailparser.
 * This is the preferred method as it handles all email formats correctly.
 */
async function parseRawEmail(rawEmail: string): Promise<InboundEmail> {
  const parsed = await simpleParser(rawEmail)

  // Handle to/from which can be AddressObject or AddressObject[]
  const toText = parsed.to
    ? Array.isArray(parsed.to)
      ? parsed.to.map((a) => a.text).join(', ')
      : parsed.to.text
    : ''
  const fromText = parsed.from?.text || ''

  return {
    to: toText,
    from: fromText,
    subject: parsed.subject || '',
    text: parsed.text || '',
    messageId: parsed.messageId || `${Date.now()}@inbox.moatless.ai`,
    ...(parsed.html ? { html: parsed.html } : {}),
    ...(parsed.inReplyTo ? { inReplyTo: parsed.inReplyTo } : {}),
    ...(parsed.references
      ? {
          references: Array.isArray(parsed.references)
            ? parsed.references.join(' ')
            : parsed.references,
        }
      : {}),
    ...(parsed.attachments && parsed.attachments.length > 0
      ? { attachments: parsed.attachments }
      : {}),
  }
}

/**
 * Parse form data into InboundEmail structure (legacy/fallback).
 * SendGrid sends email data as multipart/form-data fields in "Parsed" mode.
 */
function parseFormDataToEmail(formData: FormData): InboundEmail {
  const headers = (formData.get('headers') as string) || ''
  const htmlContent = formData.get('html') as string | null
  const inReplyTo = extractHeader(headers, 'In-Reply-To')
  const references = extractHeader(headers, 'References')

  return {
    to: (formData.get('to') as string) || '',
    from: (formData.get('from') as string) || '',
    subject: (formData.get('subject') as string) || '',
    text: (formData.get('text') as string) || '',
    messageId: extractMessageId(headers),
    ...(htmlContent ? { html: htmlContent } : {}),
    ...(inReplyTo ? { inReplyTo } : {}),
    ...(references ? { references } : {}),
  }
}

/**
 * Extract Message-ID from raw email headers.
 */
function extractMessageId(headers: string): string {
  const match = headers.match(/^Message-ID:\s*<?([^>\r\n]+)>?/im)
  return match?.[1] ?? `${Date.now()}@inbox.moatless.ai`
}

/**
 * Extract a specific header value from raw email headers.
 */
function extractHeader(headers: string, headerName: string): string | undefined {
  const regex = new RegExp(`^${headerName}:\\s*(.+)$`, 'im')
  const match = headers.match(regex)
  return match?.[1]?.trim()
}
