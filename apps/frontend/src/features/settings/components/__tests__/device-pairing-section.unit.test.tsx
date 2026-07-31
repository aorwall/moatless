import { fireEvent, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { usePairingCode } from '@/features/settings/hooks/use-pairing-code'
import { render } from '@/test-utils/render'
import { DevicePairingSection, pairingUrl } from '../device-pairing-section'

vi.mock('@/features/settings/hooks/use-pairing-code', () => ({
  usePairingCode: vi.fn(),
}))

const CODE = 'ABCDEFGHJKLM'

/** The component holds the result in state, so a rendered code can only be
 *  reached through the mutation's own success callback. */
const issues = (code = CODE) => {
  vi.mocked(usePairingCode).mockReturnValue({
    isPending: false,
    mutate: (_request: unknown, options?: { onSuccess?: (result: unknown) => void }) =>
      options?.onSuccess?.({
        id: 'code-1',
        code,
        label: null,
        expiresAt: new Date(Date.now() + 5 * 60 * 1000).toISOString(),
      }),
  } as unknown as ReturnType<typeof usePairingCode>)
}

const generate = () => fireEvent.click(screen.getByRole('button', { name: /Generate/ }))

describe('DevicePairingSection', () => {
  it('shows nothing to pair with until a code has been generated', () => {
    issues()
    render(<DevicePairingSection />)

    expect(screen.queryByText('Server address')).not.toBeInTheDocument()
    expect(document.querySelector('svg[height="176"]')).toBeNull()
  })

  /**
   * The payload behind the QR is the contract with T3 Code's scanner: it reads
   * a scan as a pairing URL and splits it back into host and code
   * (`apps/mobile/src/features/connection/pairing.ts`), so this origin and a
   * `#token=` fragment are exactly what has to be encoded — no path, and the
   * token in the fragment rather than the query.
   */
  it('encodes the pairing url T3 Code scans', () => {
    expect(pairingUrl(CODE)).toBe(`${window.location.origin}/#token=${CODE}`)
  })

  it('renders the code as a QR and, the same code, by hand', () => {
    issues()
    render(<DevicePairingSection />)
    generate()

    expect(document.querySelector('svg[height="176"]')).not.toBeNull()
    expect(screen.getByText(window.location.origin)).toBeInTheDocument()
    // Grouped for reading aloud; the server strips the dash.
    expect(screen.getByText('ABCDEF-GHJKLM')).toBeInTheDocument()
  })
})
