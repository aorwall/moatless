import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { GenericSelector } from '../generic-selector'

describe('GenericSelector - Tier 2 Error Handling Tests', () => {
  const defaultProps = {
    onChange: vi.fn(),
    placeholder: 'Select option',
    items: [
      { id: '1', label: 'Option 1' },
      { id: '2', label: 'Option 2' },
      { id: '3', label: 'Option 3' },
    ],
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Loading State', () => {
    it('shows loading text when loading', () => {
      render(<GenericSelector {...defaultProps} loading />)

      expect(screen.getByText('Loading...')).toBeInTheDocument()
    })

    it('disables selector when loading', () => {
      render(<GenericSelector {...defaultProps} loading />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toBeInTheDocument()
    })

    it('does not show placeholder when loading', () => {
      render(<GenericSelector {...defaultProps} loading />)

      expect(screen.queryByText('Select option')).not.toBeInTheDocument()
    })
  })

  describe('Error State', () => {
    it('shows error text when isError is true', () => {
      render(<GenericSelector {...defaultProps} isError />)

      expect(screen.getByText('Failed to load')).toBeInTheDocument()
    })

    it('disables selector when error', () => {
      render(<GenericSelector {...defaultProps} isError />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toBeInTheDocument()
    })

    it('applies destructive styling when error', () => {
      render(<GenericSelector {...defaultProps} isError />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toHaveClass('border-destructive', 'text-destructive')
    })
  })

  describe('Disabled State', () => {
    it('disables selector when disabled prop is true', () => {
      render(<GenericSelector {...defaultProps} disabled />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toBeInTheDocument()
    })
  })

  describe('Items Rendering', () => {
    it('renders items correctly', () => {
      render(<GenericSelector {...defaultProps} />)

      expect(screen.getByRole('combobox')).toBeInTheDocument()
    })

    it('handles empty items list', () => {
      render(<GenericSelector {...defaultProps} items={[]} />)

      expect(screen.getByRole('combobox')).toBeInTheDocument()
    })
  })

  describe('Value Selection', () => {
    it('displays selected value', () => {
      render(<GenericSelector {...defaultProps} value="1" />)

      expect(screen.getByText('Option 1')).toBeInTheDocument()
    })

    it('shows placeholder when no value', () => {
      render(<GenericSelector {...defaultProps} />)

      expect(screen.getByText('Select option')).toBeInTheDocument()
    })
  })

  describe('Custom Width', () => {
    it('applies default width', () => {
      render(<GenericSelector {...defaultProps} />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toHaveClass('w-[180px]')
    })

    it('applies custom width', () => {
      render(<GenericSelector {...defaultProps} width="w-[240px]" />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toHaveClass('w-[240px]')
    })
  })

  describe('Custom className', () => {
    it('applies custom className', () => {
      render(<GenericSelector {...defaultProps} className="custom-class" />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toHaveClass('custom-class')
    })
  })

  describe('Combined States', () => {
    it('prioritizes loading over error', () => {
      render(<GenericSelector {...defaultProps} loading isError />)

      expect(screen.getByText('Loading...')).toBeInTheDocument()
      expect(screen.queryByText('Failed to load')).not.toBeInTheDocument()
    })

    it('disables when both disabled and loading', () => {
      render(<GenericSelector {...defaultProps} disabled loading />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toBeInTheDocument()
    })

    it('shows error styling but respects disabled state', () => {
      render(<GenericSelector {...defaultProps} disabled isError />)

      const trigger = screen.getByRole('combobox')
      expect(trigger).toBeInTheDocument()
      expect(trigger).toHaveClass('border-destructive')
    })
  })
})
