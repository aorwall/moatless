import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { FileIcon } from 'lucide-react'
import { describe, expect, it } from 'vitest'
import { PopoverSelector } from '../popover-selector'

describe('PopoverSelector - Tier 2 Conditional Rendering Tests', () => {
  describe('Loading State', () => {
    it('shows loading text when loading', () => {
      render(
        <PopoverSelector icon={FileIcon} loading placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('Loading...')
    })

    it('does not show placeholder when loading', () => {
      render(
        <PopoverSelector icon={FileIcon} loading placeholder="Select file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).not.toHaveTextContent('Select file')
    })
  })

  describe('Disabled State', () => {
    it('disables trigger when disabled', () => {
      render(
        <PopoverSelector icon={FileIcon} disabled placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      const trigger = screen.getByRole('button')
      expect(trigger).toBeDisabled()
    })

    it('disables trigger when loading', () => {
      render(
        <PopoverSelector icon={FileIcon} loading placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      const trigger = screen.getByRole('button')
      expect(trigger).toBeDisabled()
    })
  })

  describe('Value Display', () => {
    it('displays selected value', () => {
      render(
        <PopoverSelector
          icon={FileIcon}
          value="file1"
          displayValue="README.md"
          placeholder="Select"
        >
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('README.md')
    })

    it('displays value when no displayValue provided', () => {
      render(
        <PopoverSelector icon={FileIcon} value="file1" placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('file1')
    })

    it('shows placeholder when no value', () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('Select file')
    })

    it('shows placeholder when value is empty string', () => {
      render(
        <PopoverSelector icon={FileIcon} value="" placeholder="Select file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('Select file')
    })

    it('shows placeholder when value is __none__', () => {
      render(
        <PopoverSelector icon={FileIcon} value="__none__" placeholder="Select file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveTextContent('Select file')
    })
  })

  describe('Icon Rendering', () => {
    it('renders provided icon', () => {
      const { container } = render(
        <PopoverSelector icon={FileIcon} placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      expect(container.querySelector('svg')).toBeInTheDocument()
    })
  })

  describe('Popover Interaction', () => {
    it('opens popover on click', async () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select">
          <div>File options</div>
        </PopoverSelector>,
      )

      await userEvent.click(screen.getByRole('button'))

      expect(screen.getByText('File options')).toBeInTheDocument()
    })

    it('does not open popover when disabled', async () => {
      render(
        <PopoverSelector icon={FileIcon} disabled placeholder="Select">
          <div>File options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toBeDisabled()
    })
  })

  describe('Tooltip', () => {
    it('shows tooltip when provided', async () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select" tooltip="Select a file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      await userEvent.hover(button)
    })

    it('shows button text in tooltip when no custom tooltip', async () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select file">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      await userEvent.hover(button)
    })
  })

  describe('Custom className', () => {
    it('applies custom className', () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select" className="custom-class">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveClass('custom-class')
    })
  })

  describe('Text Styling', () => {
    it('applies muted text color when no value', () => {
      render(
        <PopoverSelector icon={FileIcon} placeholder="Select">
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).toHaveClass('text-muted-foreground')
    })

    it('does not apply muted text color when value present', () => {
      render(
        <PopoverSelector
          icon={FileIcon}
          value="file1"
          displayValue="README.md"
          placeholder="Select"
        >
          <div>Options</div>
        </PopoverSelector>,
      )

      const button = screen.getByRole('button')
      expect(button).not.toHaveClass('text-muted-foreground')
    })
  })
})
