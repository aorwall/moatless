import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { ServerDown } from '../server-down'

describe('ServerDown', () => {
  it('renders the title and message', () => {
    render(<ServerDown onRetry={vi.fn()} />)

    expect(screen.getByText('Server unavailable')).toBeInTheDocument()
    expect(screen.getByText(/can't reach the Moatless server/i)).toBeInTheDocument()
  })

  it('calls onRetry when "Try again" is clicked', async () => {
    const user = userEvent.setup()
    const onRetry = vi.fn()

    render(<ServerDown onRetry={onRetry} />)

    await user.click(screen.getByRole('button', { name: /try again/i }))

    expect(onRetry).toHaveBeenCalledTimes(1)
  })
})
