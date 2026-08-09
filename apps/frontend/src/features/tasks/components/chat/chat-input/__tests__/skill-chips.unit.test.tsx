import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { SkillChips } from '../skill-chips'

describe('SkillChips', () => {
  it('renders one chip per activated skill', () => {
    render(<SkillChips skills={['e2e-testing', 'moat-cli']} onRemove={vi.fn()} />)

    expect(screen.getByText('e2e-testing')).toBeInTheDocument()
    expect(screen.getByText('moat-cli')).toBeInTheDocument()
  })

  it('renders nothing when no skill is activated', () => {
    const { container } = render(<SkillChips skills={[]} onRemove={vi.fn()} />)

    expect(container).toBeEmptyDOMElement()
  })

  it('removes by name', async () => {
    const onRemove = vi.fn()
    render(<SkillChips skills={['e2e-testing', 'moat-cli']} onRemove={onRemove} />)

    await userEvent.click(screen.getByRole('button', { name: 'Remove skill moat-cli' }))

    expect(onRemove).toHaveBeenCalledWith('moat-cli')
  })
})
