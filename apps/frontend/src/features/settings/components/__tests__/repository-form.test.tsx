import type { CodeExecution } from '@moatless/shared/types/repository'
import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockGithubRepository, createMockRepository } from '@/test-utils/factories'
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

      expect(screen.getByText('Create New Environment')).toBeInTheDocument()
      expect(screen.getByLabelText(/Name/)).toHaveValue('')
      expect(screen.getByLabelText(/Description/)).toHaveValue('')
      expect(screen.getByLabelText(/Repository Type/)).toHaveTextContent('Local')
      expect(screen.getByLabelText(/Execution Type/)).toHaveTextContent('Local')
    })

    it('should validate required fields', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Try to submit without filling required fields
      const submitButton = screen.getByText('Create Environment')
      fireEvent.click(submitButton)

      await waitFor(() => {
        expect(mockOnSubmit).not.toHaveBeenCalled()
      })

      // Check for validation errors
      expect(screen.queryByText(/required/i)).toBeInTheDocument()
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
            repository: {
              type: 'local',
              path: '/test/path',
            },
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
      fireEvent.click(screen.getByLabelText(/Repository Type/))
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
            repository: {
              type: 'github',
              repoId: 'owner/repo',
              branch: 'main',
            },
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
      fireEvent.click(screen.getByLabelText(/Execution Type/))
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
            repository: {
              type: 'local',
              path: '/test/path',
              useGitWorktrees: true,
            },
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

      expect(screen.getByText('Edit Environment')).toBeInTheDocument()
      expect(screen.getByLabelText(/Name/)).toHaveValue(mockRepository.name)
      expect(screen.getByLabelText(/Description/)).toHaveValue(mockRepository.description)
      expect(screen.getByLabelText(/Local Path/)).toHaveValue(
        (mockRepository.repository as LocalRepository).path,
      )
    })

    it('should populate form with GitHub environment', () => {
      const mockRepository = createMockGithubRepository()

      render(
        <RepositoryForm
          onSubmit={mockOnSubmit}
          onCancel={mockOnCancel}
          mode="edit"
          initialData={mockRepository}
        />,
      )

      expect(screen.getByLabelText(/Repository Type/)).toHaveTextContent('GitHub')
      expect(screen.getByLabelText(/Repository ID/)).toHaveValue(
        (mockRepository.repository as GithubRepository).repoId,
      )
      expect(screen.getByLabelText(/Branch/)).toHaveValue(
        (mockRepository.repository as GithubRepository).branch,
      )
      expect(screen.getByLabelText(/Execution Type/)).toHaveTextContent('Docker')
      expect(screen.getByLabelText(/Docker Image/)).toHaveValue(
        (mockRepository.codeExecution as CodeExecution).image,
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
      fireEvent.click(screen.getByText('Update Environment'))

      await waitFor(() => {
        expect(mockOnSubmit).toHaveBeenCalledWith(
          expect.objectContaining({
            name: 'Updated Environment Name',
            description: mockRepository.description,
            repository: mockRepository.repository,
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
        expect(screen.queryByText(/required/i)).toBeInTheDocument()
      })

      // Start typing in name field
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'T' },
      })

      // Error should be cleared
      await waitFor(() => {
        expect(screen.queryByText(/required/i)).not.toBeInTheDocument()
      })
    })

    it('should validate with Zod schema', async () => {
      render(<RepositoryForm onSubmit={mockOnSubmit} onCancel={mockOnCancel} mode="create" />)

      // Fill invalid data
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Test' },
      })

      // Switch to GitHub but don't fill required fields
      fireEvent.click(screen.getByLabelText(/Repository Type/))
      fireEvent.click(screen.getByText('GitHub'))

      // Try to submit
      fireEvent.click(screen.getByText('Create Repository'))

      await waitFor(() => {
        expect(mockOnSubmit).not.toHaveBeenCalled()
      })

      // Should show validation error for missing GitHub fields
      expect(screen.queryByText(/required/i)).toBeInTheDocument()
    })
  })
})
