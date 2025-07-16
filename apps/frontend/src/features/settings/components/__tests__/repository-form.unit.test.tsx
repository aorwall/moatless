import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockRepository } from '@/test-utils/factories'

import { render } from '@/test-utils/render'
import { RepositoryForm } from '../repository-form'

// Mock heavy operations to improve test performance
vi.mock('@moatless/types', () => ({
  repositoryFormSchema: {
    parse: vi.fn((data) => data),
    safeParse: vi.fn((data) => ({ success: true, data })),
  },
}))

// Mock the schema imports used in the component
vi.mock('@moatless/types', () => ({
  CreateRepositorySchema: {
    parse: vi.fn((data) => {
      // Basic validation for required fields - throw if validation fails
      if (!data.name || !data.path) {
        const error: any = new Error('Validation failed')
        error.name = 'ZodError'
        error.errors = [
          ...(!data.name ? [{ path: ['name'], message: 'Required' }] : []),
          ...(!data.path ? [{ path: ['path'], message: 'Required' }] : []),
        ]
        throw error
      }
      return data
    }),
    safeParse: vi.fn((data) => {
      // Basic validation for required fields
      if (!data.name || !data.path) {
        return {
          success: false,
          error: {
            errors: [
              ...(!data.name ? [{ path: ['name'], message: 'Required' }] : []),
              ...(!data.path ? [{ path: ['path'], message: 'Required' }] : []),
            ],
          },
        }
      }
      return { success: true, data }
    }),
  },
  UpdateRepositorySchema: {
    parse: vi.fn((data) => {
      // Basic validation for required fields - throw if validation fails
      if (!data.name || !data.path) {
        const error: any = new Error('Validation failed')
        error.name = 'ZodError'
        error.errors = [
          ...(!data.name ? [{ path: ['name'], message: 'Required' }] : []),
          ...(!data.path ? [{ path: ['path'], message: 'Required' }] : []),
        ]
        throw error
      }
      return data
    }),
    safeParse: vi.fn((data) => {
      // Basic validation for required fields
      if (!data.name || !data.path) {
        return {
          success: false,
          error: {
            errors: [
              ...(!data.name ? [{ path: ['name'], message: 'Required' }] : []),
              ...(!data.path ? [{ path: ['path'], message: 'Required' }] : []),
            ],
          },
        }
      }
      return { success: true, data }
    }),
  },
}))

// Configure shorter timeouts for faster tests
const customWaitFor = (fn: () => void, options = {}): Promise<void> =>
  waitFor(fn, { timeout: 2000, ...options })

describe('RepositoryForm', () => {
  const mockOnSubmit = vi.fn()
  const mockOnCancel = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Create Mode', () => {
    it('should render empty form in create mode', () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      expect(screen.getByText('Create New Repository')).toBeInTheDocument()
      expect(screen.getByLabelText(/Name/)).toHaveValue('')
      expect(screen.getByLabelText(/Description/)).toHaveValue('')
      expect(screen.getByRole('combobox', { name: /repository type/i })).toHaveTextContent('Local')
      expect(screen.getByRole('combobox', { name: /code execution type/i })).toHaveTextContent(
        'Local',
      )
    })

    it('should submit valid local environment', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Fill in the form
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Test Environment' },
      })
      fireEvent.change(screen.getByLabelText(/Description/), {
        target: { value: 'Test description' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/path' },
      })

      // Submit the form
      fireEvent.click(screen.getByText('Create Repository'))

      await customWaitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            name: 'Test Environment',
            description: 'Test description',
            path: '/test/path',
            codeExecution: {
              type: 'local',
            },
          }),
        )
      })
    })

    it('should toggle git worktrees option', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Fill required fields
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Worktree Test' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/path' },
      })

      // Toggle git worktrees
      const worktreeSwitch = screen.getByLabelText(/Use Git Worktrees/)
      fireEvent.click(worktreeSwitch)

      // Submit
      fireEvent.click(screen.getByText('Create Repository'))

      await customWaitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            path: '/test/path',
            useGitWorktrees: true,
          }),
        )
      })
    })
  })

  describe('Edit Mode', () => {
    it('should populate form with existing data', () => {
      const mockRepository = createMockRepository()

      render(
        <RepositoryForm
          onSubmit={mockOnSubmit}
          onCancel={mockOnCancel}
          mode="edit"
          initialData={mockRepository}
        />,
      )

      expect(screen.getByText('Edit Repository')).toBeInTheDocument()
      expect(screen.getByLabelText(/Name/)).toHaveValue(mockRepository.name)
      expect(screen.getByLabelText(/Description/)).toHaveValue(mockRepository.description)
      expect(screen.getByLabelText(/Local Path/)).toHaveValue(mockRepository.path)
    })

    it('should submit updated data', async () => {
      const mockRepository = createMockRepository()

      render(
        <RepositoryForm
          onSubmit={mockOnSubmit}
          onCancel={mockOnCancel}
          mode="edit"
          initialData={mockRepository}
        />,
      )

      // Update name
      const nameInput = screen.getByLabelText(/Name/)
      fireEvent.change(nameInput, {
        target: { value: 'Updated Environment Name' },
      })

      // Submit
      fireEvent.click(screen.getByText('Update Repository'))

      await customWaitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            name: 'Updated Environment Name',
            description: mockRepository.description,
            path: mockRepository.path,
            codeExecution: mockRepository.codeExecution,
          }),
        )
      })
    })
  })

  describe('Common Interactions', () => {
    it('should call onCancel when cancel button is clicked', () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      fireEvent.click(screen.getByText('Cancel'))
      expect(mockOnCancel).toHaveBeenCalled()
    })

    it('should disable submit button when submitting', () => {
      render(
        <RepositoryForm
          onSubmit={mockOnSubmit}
          onCancel={mockOnCancel}
          mode="create"
          isSubmitting={true}
        />,
      )

      const submitButton = screen.getByText('Creating...')
      expect(submitButton).toBeDisabled()
    })
  })
})
