import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { GitHubPatForm } from '../github-pat-form'

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

function input() {
  return screen.getByPlaceholderText(/personal access token|new token/i)
}

describe('GitHubPatForm', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('disables save until a non-whitespace token is entered', () => {
    render(<GitHubPatForm hasToken={false} onSave={vi.fn()} isSaving={false} />)
    const save = screen.getByRole('button', { name: 'Save' })

    expect(save).toBeDisabled()
    fireEvent.change(input(), { target: { value: '   ' } })
    expect(save).toBeDisabled()
    fireEvent.change(input(), { target: { value: 'ghp_x' } })
    expect(save).toBeEnabled()
  })

  it('passes the trimmed token to onSave', async () => {
    const onSave = vi.fn().mockResolvedValue({})
    render(<GitHubPatForm hasToken={false} onSave={onSave} isSaving={false} />)

    fireEvent.change(input(), { target: { value: '  ghp_x  ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => expect(onSave).toHaveBeenCalledWith('ghp_x'))
  })

  it('clears the field after a successful save', async () => {
    render(
      <GitHubPatForm hasToken={false} onSave={vi.fn().mockResolvedValue({})} isSaving={false} />,
    )

    fireEvent.change(input(), { target: { value: 'ghp_x' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => expect(input()).toHaveValue(''))
  })

  it('keeps the token in the field when the save fails, so it can be retried', async () => {
    const onSave = vi.fn().mockRejectedValue(new Error('invalid token'))
    render(<GitHubPatForm hasToken={false} onSave={onSave} isSaving={false} />)

    fireEvent.change(input(), { target: { value: 'ghp_x' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => expect(onSave).toHaveBeenCalled())
    expect(input()).toHaveValue('ghp_x')
  })

  it('shows Update instead of Save when a token already exists', () => {
    render(<GitHubPatForm hasToken onSave={vi.fn()} isSaving={false} />)
    expect(screen.getByRole('button', { name: 'Update' })).toBeInTheDocument()
  })

  describe('remove control', () => {
    it('is offered only when a token exists', () => {
      const { rerender } = render(
        <GitHubPatForm hasToken={false} onSave={vi.fn()} isSaving={false} onRemove={vi.fn()} />,
      )
      expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument()

      rerender(<GitHubPatForm hasToken onSave={vi.fn()} isSaving={false} onRemove={vi.fn()} />)
      expect(screen.getByRole('button', { name: 'Remove' })).toBeInTheDocument()
    })

    it('is absent when no remove handler is supplied', () => {
      render(<GitHubPatForm hasToken onSave={vi.fn()} isSaving={false} />)
      expect(screen.queryByRole('button', { name: /remove/i })).not.toBeInTheDocument()
    })
  })
})
