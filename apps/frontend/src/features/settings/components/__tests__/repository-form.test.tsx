import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockRepository } from '@/test-utils/factories'

import { render } from '@/test-utils/render'
import { RepositoryForm } from '../repository-form'

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
      expect(screen.getByRole('combobox', { name: /code execution type/i })).toHaveTextContent('Local')
    })

    it('should validate required fields', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Try to submit without filling required fields
      const submitButton = screen.getByText('Create Repository')
      fireEvent.click(submitButton)

      await waitFor(() => {
        expect(mockOnSubmit).not.toHaveBeenCalled()
      })

      // Check for validation errors - Zod default message
      expect(screen.getByText('Required')).toBeInTheDocument()
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

      await waitFor(() => {
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

    it('should handle GitHub repository configuration', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Switch to GitHub repository
      fireEvent.click(screen.getByRole('combobox', { name: /repository type/i }))
      fireEvent.click(screen.getByText('GitHub'))

      // GitHub-specific fields should appear
      expect(screen.getByLabelText(/Repository ID/)).toBeInTheDocument()
      expect(screen.getByLabelText(/Branch/)).toBeInTheDocument()
      expect(screen.getByLabelText(/Access Token/)).toBeInTheDocument()

      // Fill GitHub fields
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'GitHub Test' },
      })
      fireEvent.change(screen.getByLabelText(/Repository ID/), {
        target: { value: 'owner/repo' },
      })
      fireEvent.change(screen.getByLabelText(/Branch/), {
        target: { value: 'main' },
      })

      // Submit
      fireEvent.click(screen.getByText('Create Repository'))

      await waitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            path: '',
          }),
        )
      })
    })

    it('should handle Docker execution configuration', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Fill basic fields
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Docker Test' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/path' },
      })

      // Switch to Docker execution
      fireEvent.click(screen.getByRole('combobox', { name: /code execution type/i }))
      fireEvent.click(screen.getByText('Docker'))

      // Docker-specific fields should appear
      expect(screen.getByLabelText(/Docker Image/)).toBeInTheDocument()

      // Fill Docker fields
      fireEvent.change(screen.getByLabelText(/Docker Image/), {
        target: { value: 'node:18-alpine' },
      })

      // Submit
      fireEvent.click(screen.getByText('Create Repository'))

      await waitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            codeExecution: {
              type: 'docker',
              image: 'node:18-alpine',
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

      await waitFor(() => {
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
      expect(screen.getByLabelText(/Local Path/)).toHaveValue(
        mockRepository.path,
      )
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

      await waitFor(() => {
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

    it('should clear errors when user types', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Submit without name to trigger error
      fireEvent.click(screen.getByText('Create Repository'))

      await waitFor(() => {
        expect(screen.getByText('Required')).toBeInTheDocument()
      })

      // Start typing in name field
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'T' },
      })

      // Error should be cleared
      await waitFor(() => {
        expect(screen.queryByText('Required')).not.toBeInTheDocument()
      })
    })

    it('should validate with Zod schema', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Try to submit without required fields
      fireEvent.click(screen.getByText('Create Repository'))

      await waitFor(() => {
        expect(mockOnSubmit).not.toHaveBeenCalled()
      })

      // Should show validation error for missing required fields
      expect(screen.getByText('Required')).toBeInTheDocument()
    })
  })
})
