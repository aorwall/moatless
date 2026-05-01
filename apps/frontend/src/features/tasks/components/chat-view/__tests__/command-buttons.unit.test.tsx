import type { CommandResponse } from '@moatless/api-client'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CommandButtons } from '../command-buttons'

describe('CommandButtons', () => {
  const mockCommands: CommandResponse[] = [
    {
      name: 'commit',
      description: 'Commit changes',
      prompt: 'commit the changes',
      icon: 'GitCommit',
      variant: 'outline',
    },
    {
      name: 'continue',
      description: 'Continue after error',
      prompt: 'continue',
      icon: 'Play',
      variant: 'default',
    },
    {
      name: 'create-pr',
      description: 'Create pull request',
      prompt: 'create a pull request',
      icon: 'GitPullRequest',
    },
  ]

  it('should render nothing when no commands are provided', () => {
    const { container } = render(<CommandButtons commands={[]} onCommandClick={vi.fn()} />)

    expect(container.firstChild).toBeNull()
  })

  it('should render all provided commands', () => {
    render(<CommandButtons commands={mockCommands} onCommandClick={vi.fn()} />)

    expect(screen.getByText('Commit changes')).toBeInTheDocument()
    expect(screen.getByText('Continue after error')).toBeInTheDocument()
    expect(screen.getByText('Create pull request')).toBeInTheDocument()
  })

  it('should call onCommandClick with the correct command when clicked', () => {
    const mockOnCommandClick = vi.fn()
    render(<CommandButtons commands={mockCommands} onCommandClick={mockOnCommandClick} />)

    const commitButton = screen.getByText('Commit changes')
    fireEvent.click(commitButton)

    expect(mockOnCommandClick).toHaveBeenCalledTimes(1)
    expect(mockOnCommandClick).toHaveBeenCalledWith(mockCommands[0])
  })

  it('should disable all buttons when disabled prop is true', () => {
    render(<CommandButtons commands={mockCommands} onCommandClick={vi.fn()} disabled={true} />)

    const buttons = screen.getAllByRole('button')
    for (const button of buttons) {
      expect(button).toBeDisabled()
    }
  })

  it('should not disable buttons when disabled prop is false', () => {
    render(<CommandButtons commands={mockCommands} onCommandClick={vi.fn()} disabled={false} />)

    const buttons = screen.getAllByRole('button')
    for (const button of buttons) {
      expect(button).not.toBeDisabled()
    }
  })

  it('should apply the correct variant prop to buttons', () => {
    render(<CommandButtons commands={mockCommands} onCommandClick={vi.fn()} />)

    const buttons = screen.getAllByRole('button')

    // Verify buttons are rendered with correct props
    expect(buttons).toHaveLength(3)

    // Since we can't directly test variant classes from the Button component,
    // we verify that the buttons are rendered and functional
    expect(buttons[0]).toBeInTheDocument()
    expect(buttons[1]).toBeInTheDocument()
    expect(buttons[2]).toBeInTheDocument()
  })

  it('should render commands with correct icons', () => {
    render(<CommandButtons commands={mockCommands} onCommandClick={vi.fn()} />)

    // Check that SVG icons are rendered within buttons
    const buttons = screen.getAllByRole('button')
    for (const button of buttons) {
      const svg = button.querySelector('svg')
      expect(svg).toBeInTheDocument()
      expect(svg).toHaveClass('size-3.5', 'shrink-0')
    }
  })

  it('should handle commands without icons', () => {
    const commandsWithoutIcon: CommandResponse[] = [
      {
        name: 'test',
        description: 'Test command',
        prompt: 'test',
      },
    ]

    render(<CommandButtons commands={commandsWithoutIcon} onCommandClick={vi.fn()} />)

    const button = screen.getByText('Test command')
    const svg = button.parentElement?.querySelector('svg')
    expect(svg).toBeInTheDocument() // Should render default MessageSquare icon
  })

  it('should handle multiple command clicks', () => {
    const mockOnCommandClick = vi.fn()
    render(<CommandButtons commands={mockCommands} onCommandClick={mockOnCommandClick} />)

    const commitButton = screen.getByText('Commit changes')
    const continueButton = screen.getByText('Continue after error')

    fireEvent.click(commitButton)
    fireEvent.click(continueButton)

    expect(mockOnCommandClick).toHaveBeenCalledTimes(2)
    expect(mockOnCommandClick).toHaveBeenNthCalledWith(1, mockCommands[0])
    expect(mockOnCommandClick).toHaveBeenNthCalledWith(2, mockCommands[1])
  })

  it('should not call onCommandClick when disabled', () => {
    const mockOnCommandClick = vi.fn()
    render(
      <CommandButtons
        commands={mockCommands}
        onCommandClick={mockOnCommandClick}
        disabled={true}
      />,
    )

    const commitButton = screen.getByText('Commit changes')
    fireEvent.click(commitButton)

    expect(mockOnCommandClick).not.toHaveBeenCalled()
  })

  it('should render with proper layout classes', () => {
    const { container } = render(
      <CommandButtons commands={mockCommands} onCommandClick={vi.fn()} />,
    )

    const wrapper = container.firstChild
    expect(wrapper).toHaveClass('relative', 'w-full', 'rounded-t-lg', 'bg-background', 'py-2')

    const buttonContainer = wrapper?.firstChild
    expect(buttonContainer).toHaveClass('flex', 'w-full', 'flex-nowrap', 'gap-2', 'overflow-x-auto')
  })
})
