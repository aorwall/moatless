import { customInstance } from '@moatless/api-client/custom-instance'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { getErrorMessage } from '@/lib/utils/error'
import { render, screen } from '@/test-utils/render'

/**
 * A refusal the backend explains in the body, delivered the way an HTTP/2
 * deployment delivers it: `statusText` is empty, because HTTP/2 dropped the
 * reason phrase. Everything the screen can say has to come out of the body.
 */
function refuse(body: unknown, status = 400) {
  vi.spyOn(globalThis, 'fetch').mockResolvedValue(
    new Response(JSON.stringify(body), {
      status,
      statusText: '',
      headers: { 'content-type': 'application/json' },
    }),
  )
}

async function failedRequest(): Promise<unknown> {
  try {
    await customInstance('/api/v1/admin/settings/github/apps', { method: 'POST' })
  } catch (error) {
    return error
  }
  throw new Error('the request was expected to fail')
}

const REFUSAL = 'GitHub has no App with ID 150038583. Nothing was stored.'

describe('failed API requests', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('carries the server explanation rather than an empty status line', async () => {
    refuse({ error: REFUSAL })

    const error = await failedRequest()

    expect(error).toBeInstanceOf(Error)
    expect((error as Error).message).toBe(REFUSAL)
  })

  it('shows that explanation on screen', async () => {
    refuse({ error: REFUSAL })

    render(<ErrorAlert error={await failedRequest()} />)

    expect(screen.getByText(REFUSAL)).toBeInTheDocument()
  })

  it('shows it in a toast description too', async () => {
    refuse({ error: REFUSAL })

    expect(getErrorMessage(await failedRequest())).toBe(REFUSAL)
  })

  it('falls back to the status when the body explains nothing', async () => {
    refuse({}, 502)

    expect((await failedRequest()) as Error).toHaveProperty(
      'message',
      'Request failed with status 502',
    )
  })
})
