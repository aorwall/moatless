import { fireEvent, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import { render } from '@/test-utils/render'

const submit = vi.fn()

vi.mock('@/features/feedback/api/feedback.api', () => ({
  feedbackApi: {
    submit: (...args: unknown[]) => submit(...args),
    listForTask: vi.fn().mockResolvedValue({ items: [] }),
  },
}))

describe('FeedbackDialog', () => {
  beforeEach(() => {
    submit.mockReset()
  })

  it('disables submit when comment is empty and shows error', async () => {
    submit.mockResolvedValue({ feedbackId: 'fb-1' })

    render(<FeedbackDialog open onOpenChange={vi.fn()} taskId="task-1" surface="task" />)

    const submitBtn = screen.getByRole('button', { name: /submit/i })
    await userEvent.click(submitBtn)
    await screen.findByText(/please enter a comment/i)
    expect(submit).not.toHaveBeenCalled()
  })

  it('submits the typed comment and closes the dialog', async () => {
    submit.mockResolvedValue({
      feedbackId: 'fb-1',
      taskId: 'task-1',
      surface: 'task',
      source: 'user',
      comment: 'great',
      createdAt: '',
      createdBy: '',
    })
    const onOpenChange = vi.fn()

    render(
      <FeedbackDialog
        open
        onOpenChange={onOpenChange}
        taskId="task-1"
        messageUuid="msg-1"
        toolUseId="tool-1"
        surface="tool_call"
        metadata={{ note: 'x' }}
      />,
    )

    const textarea = screen.getByLabelText(/comment/i)
    fireEvent.change(textarea, { target: { value: '  needs better skill  ' } })

    const submitBtn = screen.getByRole('button', { name: /submit/i })
    await userEvent.click(submitBtn)

    await waitFor(() => {
      expect(submit).toHaveBeenCalledTimes(1)
    })
    expect(submit).toHaveBeenCalledWith({
      taskId: 'task-1',
      messageUuid: 'msg-1',
      toolUseId: 'tool-1',
      surface: 'tool_call',
      comment: 'needs better skill',
      metadata: { note: 'x' },
    })
    await waitFor(() => {
      expect(onOpenChange).toHaveBeenCalledWith(false)
    })
  })

  it('mounts nothing while closed and opens after first open', async () => {
    const { rerender, container } = render(
      <FeedbackDialog open={false} onOpenChange={vi.fn()} taskId="task-1" surface="task" />,
    )

    // Lazy mount: a never-opened dialog contributes nothing to the tree.
    expect(container).toBeEmptyDOMElement()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

    rerender(<FeedbackDialog open onOpenChange={vi.fn()} taskId="task-1" surface="task" />)
    expect(await screen.findByRole('dialog')).toBeInTheDocument()

    // Closing again keeps it mounted (exit animation) but hidden.
    rerender(<FeedbackDialog open={false} onOpenChange={vi.fn()} taskId="task-1" surface="task" />)
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })
})
