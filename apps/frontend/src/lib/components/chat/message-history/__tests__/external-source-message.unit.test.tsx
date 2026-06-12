import type { UiMessage } from '@moatless/api-client'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { ExternalSourceMessageComponent } from '../external-source-message'

const createChannelMessage = (
  adapterKind: string,
  content: string,
  overrides?: Partial<UiMessage>,
): UiMessage => ({
  uuid: 'msg-1',
  role: 'user',
  content: [{ type: 'text', text: content }],
  toolResults: [],
  messageType: 'user',
  timestamp: new Date().toISOString(),
  externalSource: { adapterKind },
  ...overrides,
})

const longContent = Array.from({ length: 10 }, (_, i) => `line ${i + 1}`).join('\n')

describe('ExternalSourceMessageComponent', () => {
  describe('adapter labels', () => {
    it('renders "via GitHub PR" for the github_pr adapter', () => {
      render(<ExternalSourceMessageComponent message={createChannelMessage('github_pr', 'hi')} />)

      expect(screen.getByText('via GitHub PR')).toBeInTheDocument()
    })

    it('renders "via Linear" for the linear adapter', () => {
      render(<ExternalSourceMessageComponent message={createChannelMessage('linear', 'hi')} />)

      expect(screen.getByText('via Linear')).toBeInTheDocument()
    })

    it('renders "via Slack" for the slack adapter', () => {
      render(<ExternalSourceMessageComponent message={createChannelMessage('slack', 'hi')} />)

      expect(screen.getByText('via Slack')).toBeInTheDocument()
    })

    it('falls back to "via Integration" for unknown adapters', () => {
      render(<ExternalSourceMessageComponent message={createChannelMessage('mystery', 'hi')} />)

      expect(screen.getByText('via Integration')).toBeInTheDocument()
    })
  })

  describe('expand/collapse', () => {
    it('collapses long content by default and expands on toggle', async () => {
      const user = userEvent.setup()
      render(
        <ExternalSourceMessageComponent message={createChannelMessage('github_pr', longContent)} />,
      )

      const body = screen.getByText(/line 1/)
      expect(body).toHaveClass('line-clamp-4')

      await user.click(screen.getByRole('button', { name: /show more/i }))
      expect(body).not.toHaveClass('line-clamp-4')

      await user.click(screen.getByRole('button', { name: /show less/i }))
      expect(body).toHaveClass('line-clamp-4')
    })

    it('treats few-line but very long content as collapsible', () => {
      const message = createChannelMessage('slack', 'x'.repeat(500))
      render(<ExternalSourceMessageComponent message={message} />)

      expect(screen.getByRole('button', { name: /show more/i })).toBeInTheDocument()
    })

    it('renders short content without a toggle or clamp', () => {
      render(<ExternalSourceMessageComponent message={createChannelMessage('slack', 'short')} />)

      expect(screen.queryByRole('button')).not.toBeInTheDocument()
      expect(screen.getByText('short')).not.toHaveClass('line-clamp-4')
    })

    it('also collapses long content in the task variant', async () => {
      const user = userEvent.setup()
      const message = createChannelMessage('github_pr', longContent, {
        externalSource: undefined,
        sourceTaskId: 'task-42',
      })
      render(<ExternalSourceMessageComponent message={message} variant="task" />)

      expect(screen.getByText('from task')).toBeInTheDocument()
      expect(screen.getByText(/line 1/)).toHaveClass('line-clamp-4')

      await user.click(screen.getByRole('button', { name: /show more/i }))
      expect(screen.getByText(/line 1/)).not.toHaveClass('line-clamp-4')
    })
  })
})
