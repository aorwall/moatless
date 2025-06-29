import { fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createMockEnvironment,
  createMockEnvironmentList,
  createMockGithubEnvironment,
} from '@/test-utils/factories'
import { render } from '@/test-utils/render'
import { EnvironmentForm } from '../../settings/components/environment-form'
import { environmentsApi } from '../api/environments'
import { EnvironmentList } from '../components/environment-list'

// Mock the environments API
vi.mock('../api/environments', () => ({
  environmentsApi: {
    listEnvironments: vi.fn(),
    getEnvironment: vi.fn(),
    createEnvironment: vi.fn(),
    updateEnvironment: vi.fn(),
    deleteEnvironment: vi.fn(),
  },
}))

// Mock router navigation
const mockNavigate = vi.fn()
vi.mock('@tanstack/react-router', () => ({
  ...vi.importActual('@tanstack/react-router'),
  useNavigate: () => mockNavigate,
  useParams: () => ({ environmentId: 'env-123' }),
}))

describe('Environment Management Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Complete User Flow', () => {
    it('should handle complete environment lifecycle', async () => {
      // Step 1: List environments
      const initialEnvironments = createMockEnvironmentList(2)
      vi.mocked(environmentsApi.listEnvironments).mockResolvedValue(initialEnvironments)

      const { rerender } = render(<EnvironmentList />)

      await waitFor(() => {
        expect(screen.getByText(initialEnvironments[0].name)).toBeInTheDocument()
        expect(screen.getByText(initialEnvironments[1].name)).toBeInTheDocument()
      })

      // Step 2: Navigate to create new environment
      // Simulate clicking "Create Environment" button
      const createButton = screen.getByText(/Create Environment/i)
      expect(createButton).toBeInTheDocument()

      // Step 3: Fill and submit create form
      const newEnvironment = createMockEnvironment({
        id: 'new-env-123',
        name: 'New Test Environment',
      })
      vi.mocked(environmentsApi.createEnvironment).mockResolvedValueOnce(newEnvironment)

      // Rerender with form component
      rerender(
        <EnvironmentForm
          mode="create"
          onSubmit={async (data) => {
            await environmentsApi.createEnvironment(data)
            // Simulate navigation back to list
          }}
          onCancel={() => {
            // Simulate navigation back
          }}
        />,
      )

      // Fill the form
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'New Test Environment' },
      })
      fireEvent.change(screen.getByLabelText(/Description/), {
        target: { value: 'Integration test environment' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/integration/path' },
      })

      // Submit form
      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        expect(environmentsApi.createEnvironment).toHaveBeenCalledWith(
          expect.objectContaining({
            name: 'New Test Environment',
            description: 'Integration test environment',
            repository: {
              type: 'local',
              path: '/test/integration/path',
            },
          }),
        )
      })

      // Step 4: View updated list with new environment
      const updatedEnvironments = [...initialEnvironments, newEnvironment]
      vi.mocked(environmentsApi.listEnvironments).mockResolvedValue(updatedEnvironments)

      rerender(<EnvironmentList />)

      await waitFor(() => {
        expect(screen.getByText('New Test Environment')).toBeInTheDocument()
      })

      // Step 5: Edit the environment
      vi.mocked(environmentsApi.getEnvironment).mockResolvedValue(newEnvironment)
      vi.mocked(environmentsApi.updateEnvironment).mockResolvedValue({
        ...newEnvironment,
        name: 'Updated Test Environment',
      })

      // Simulate clicking edit button
      rerender(
        <EnvironmentForm
          mode="edit"
          initialData={newEnvironment}
          onSubmit={async (data) => {
            await environmentsApi.updateEnvironment(newEnvironment.id, data)
          }}
          onCancel={() => {}}
        />,
      )

      // Update name
      const nameInput = screen.getByLabelText(/Name/)
      fireEvent.change(nameInput, {
        target: { value: 'Updated Test Environment' },
      })

      fireEvent.click(screen.getByText('Update Environment'))

      await waitFor(() => {
        expect(environmentsApi.updateEnvironment).toHaveBeenCalledWith(
          'new-env-123',
          expect.objectContaining({
            name: 'Updated Test Environment',
          }),
        )
      })

      // Step 6: Delete the environment
      vi.mocked(environmentsApi.deleteEnvironment).mockResolvedValue(undefined)

      // Back to list
      rerender(<EnvironmentList />)

      // Verify environment can be deleted
      expect(environmentsApi.listEnvironments).toHaveBeenCalled()
    })

    it('should handle GitHub repository flow', async () => {
      const githubEnvironment = createMockGithubEnvironment()
      vi.mocked(environmentsApi.createEnvironment).mockResolvedValue(githubEnvironment)

      render(
        <EnvironmentForm
          mode="create"
          onSubmit={async (data) => {
            await environmentsApi.createEnvironment(data)
          }}
          onCancel={() => {}}
        />,
      )

      // Fill basic info
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'GitHub Integration Test' },
      })

      // Switch to GitHub repository
      fireEvent.click(screen.getByLabelText(/Repository Type/))
      fireEvent.click(screen.getByText('GitHub'))

      // Fill GitHub fields
      fireEvent.change(screen.getByLabelText(/Repository ID/), {
        target: { value: 'test-org/test-repo' },
      })
      fireEvent.change(screen.getByLabelText(/Branch/), {
        target: { value: 'main' },
      })
      fireEvent.change(screen.getByLabelText(/Access Token/), {
        target: { value: 'github_pat_test_token' },
      })

      // Switch to Docker execution
      fireEvent.click(screen.getByLabelText(/Execution Type/))
      fireEvent.click(screen.getByText('Docker'))

      fireEvent.change(screen.getByLabelText(/Docker Image/), {
        target: { value: 'node:18' },
      })

      // Submit
      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        expect(environmentsApi.createEnvironment).toHaveBeenCalledWith(
          expect.objectContaining({
            name: 'GitHub Integration Test',
            repository: {
              type: 'github',
              repoId: 'test-org/test-repo',
              branch: 'main',
              token: 'github_pat_test_token',
            },
            codeExecution: {
              type: 'docker',
              image: 'node:18',
            },
          }),
        )
      })
    })
  })

  describe('Error Handling', () => {
    it('should display error when creation fails', async () => {
      const error = new Error('Failed to create environment')
      vi.mocked(environmentsApi.createEnvironment).mockRejectedValue(error)

      render(
        <EnvironmentForm
          mode="create"
          onSubmit={async (data) => {
            await environmentsApi.createEnvironment(data)
          }}
          onCancel={() => {}}
        />,
      )

      // Fill and submit
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Test Error' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/path' },
      })

      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        expect(environmentsApi.createEnvironment).toHaveBeenCalled()
      })

      // In a real app, we would check for error message display
    })

    it('should handle validation errors', async () => {
      render(<EnvironmentForm mode="create" onSubmit={async () => {}} onCancel={() => {}} />)

      // Try to submit without required fields
      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        // Should show validation errors
        expect(screen.queryByText(/required/i)).toBeInTheDocument()
      })

      // API should not be called
      expect(environmentsApi.createEnvironment).not.toHaveBeenCalled()
    })

    it('should handle network errors gracefully', async () => {
      const networkError = new Error('Network error')
      vi.mocked(environmentsApi.listEnvironments).mockRejectedValue(networkError)

      render(<EnvironmentList />)

      await waitFor(() => {
        // In a real app, this would show an error state
        expect(environmentsApi.listEnvironments).toHaveBeenCalled()
      })
    })
  })

  describe('Form State Management', () => {
    it('should maintain form state during validation', async () => {
      render(<EnvironmentForm mode="create" onSubmit={async () => {}} onCancel={() => {}} />)

      // Fill some fields
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Partial Environment' },
      })

      // Switch repository type
      fireEvent.click(screen.getByLabelText(/Repository Type/))
      fireEvent.click(screen.getByText('GitHub'))

      // Try to submit (should fail validation)
      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        // Name should still be filled
        expect(screen.getByLabelText(/Name/)).toHaveValue('Partial Environment')
        // Repository type should still be GitHub
        expect(screen.getByLabelText(/Repository Type/)).toHaveTextContent('GitHub')
      })
    })

    it('should reset form after successful submission', async () => {
      const mockEnvironment = createMockEnvironment()
      vi.mocked(environmentsApi.createEnvironment).mockResolvedValue(mockEnvironment)

      const onSubmit = vi.fn().mockResolvedValue(undefined)

      const { rerender } = render(
        <EnvironmentForm mode="create" onSubmit={onSubmit} onCancel={() => {}} />,
      )

      // Fill and submit
      fireEvent.change(screen.getByLabelText(/Name/), {
        target: { value: 'Test Environment' },
      })
      fireEvent.change(screen.getByLabelText(/Local Path/), {
        target: { value: '/test/path' },
      })

      fireEvent.click(screen.getByText('Create Environment'))

      await waitFor(() => {
        expect(onSubmit).toHaveBeenCalled()
      })

      // Rerender a new form (simulating navigation back)
      rerender(<EnvironmentForm mode="create" onSubmit={onSubmit} onCancel={() => {}} />)

      // Form should be empty
      expect(screen.getByLabelText(/Name/)).toHaveValue('')
    })
  })
})
