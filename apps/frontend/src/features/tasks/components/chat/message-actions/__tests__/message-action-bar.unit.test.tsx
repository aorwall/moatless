import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Copy, MessageSquarePlus } from 'lucide-react'
import { describe, expect, it, vi } from 'vitest'
import type { MessageAction } from '../message-action-bar'
import { MessageActionBar } from '../message-action-bar'

describe('MessageActionBar', () => {
  it('renders nothing when actions is empty', () => {
    const { container } = render(<MessageActionBar actions={[]} />)
    expect(container.firstChild).toBeNull()
  })

  it('renders one button per action with aria-label', () => {
    const actions: MessageAction[] = [
      { key: 'copy', icon: Copy, label: 'Copy message', onClick: vi.fn() },
      { key: 'feedback', icon: MessageSquarePlus, label: 'Give feedback', onClick: vi.fn() },
    ]
    render(<MessageActionBar actions={actions} />)
    expect(screen.getByLabelText('Copy message')).toBeDefined()
    expect(screen.getByLabelText('Give feedback')).toBeDefined()
  })

  it('invokes onClick when its button is clicked', async () => {
    const onCopy = vi.fn()
    const actions: MessageAction[] = [
      { key: 'copy', icon: Copy, label: 'Copy message', onClick: onCopy },
    ]
    render(<MessageActionBar actions={actions} />)
    await userEvent.click(screen.getByLabelText('Copy message'))
    expect(onCopy).toHaveBeenCalledTimes(1)
  })
})
