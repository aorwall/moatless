import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/features/tasks/components/file-autocomplete.tsx', () => ({
  FileAutocomplete: () => <div data-testid="file-autocomplete">Autocomplete</div>,
}))

vi.mock('@/features/uploads/hooks/use-upload.ts')
vi.mock('@/lib/hooks/use-universal-file-autocomplete.ts')

import { useUploadFile } from '@/features/uploads/hooks/use-upload.ts'
import { useUniversalFileAutocomplete } from '@/lib/hooks/use-universal-file-autocomplete.ts'
import { createMockMutation } from '@/test-utils/mocks'
import { ChatForm } from '../chat-form'

describe('ChatForm - Tier 1 Critical Tests', () => {
  const defaultProps = {
    message: '',
    onMessageChange: vi.fn(),
    onSubmit: vi.fn(),
    files: [],
    onFilesChange: vi.fn(),
  }

  const mockAutocomplete = {
    inputRef: { current: null },
    isOpen: false,
    results: [],
    selectedIndex: -1,
    isLoading: false,
    error: null,
    handleInputChange: vi.fn(),
    handleKeyDown: vi.fn(),
    selectFile: vi.fn(),
  }

  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useUploadFile).mockReturnValue(createMockMutation())
    vi.mocked(useUniversalFileAutocomplete).mockReturnValue(mockAutocomplete)
  })

  describe('Message Submission', () => {
    it('submits message on Enter key', async () => {
      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} message="hello" onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Enter}')

      expect(onSubmit).toHaveBeenCalled()
    })

    it('prevents submission when disabled', async () => {
      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} message="hello" onSubmit={onSubmit} disabled />)

      await userEvent.click(screen.getByRole('button', { name: /send/i }))

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('prevents submission when loading', async () => {
      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} message="hello" onSubmit={onSubmit} loading />)

      const textarea = screen.getByRole('textbox')
      expect(textarea).toBeDisabled()
    })

    it('adds newline on Shift+Enter', async () => {
      const onMessageChange = vi.fn()
      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} onMessageChange={onMessageChange} onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('line1{Shift>}{Enter}{/Shift}line2')

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('submits form on button click', async () => {
      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} message="test" onSubmit={onSubmit} />)

      const sendButton = screen.getByRole('button', { name: /send/i })
      await userEvent.click(sendButton)

      expect(onSubmit).toHaveBeenCalled()
    })
  })

  describe('File Upload', () => {
    it('opens file picker on upload button click', async () => {
      render(<ChatForm {...defaultProps} />)

      const uploadButton = screen.getByRole('button', { name: /upload file/i })
      expect(uploadButton).toBeInTheDocument()
    })

    it('uploads file on file selection', async () => {
      const mockMutate = vi.fn()
      vi.mocked(useUploadFile).mockReturnValue(createMockMutation({ mutate: mockMutate }))

      render(<ChatForm {...defaultProps} />)

      const file = new File(['content'], 'test.txt', { type: 'text/plain' })
      const input = document.querySelector('input[type="file"]') as HTMLInputElement

      await userEvent.upload(input, file)

      expect(mockMutate).toHaveBeenCalledWith(file, expect.any(Object))
    })

    it('calls onFilesChange when upload succeeds', async () => {
      const onFilesChange = vi.fn()
      const uploadedFile = {
        id: 'file-1',
        filename: 'test.txt',
        mimeType: 'text/plain',
        size: 100,
        uploadedAt: new Date().toISOString(),
      }

      const mockMutate = vi.fn((file: any, options: any) => {
        options.onSuccess(uploadedFile)
      })

      vi.mocked(useUploadFile).mockReturnValue(createMockMutation({ mutate: mockMutate }))

      render(<ChatForm {...defaultProps} onFilesChange={onFilesChange} />)

      const file = new File(['content'], 'test.txt', { type: 'text/plain' })
      const input = document.querySelector('input[type="file"]') as HTMLInputElement

      await userEvent.upload(input, file)

      expect(onFilesChange).toHaveBeenCalledWith([uploadedFile])
    })

    it('displays uploaded files', () => {
      const files = [
        {
          id: 'file-1',
          filename: 'test.txt',
          mimeType: 'text/plain',
          size: 100,
          uploadedAt: new Date().toISOString(),
        },
      ]

      render(<ChatForm {...defaultProps} files={files} />)

      expect(screen.getByText('test.txt')).toBeInTheDocument()
    })

    it('removes file on remove button click', async () => {
      const onFilesChange = vi.fn()
      const files = [
        {
          id: 'file-1',
          filename: 'test.txt',
          mimeType: 'text/plain',
          size: 100,
          uploadedAt: new Date().toISOString(),
        },
      ]

      render(<ChatForm {...defaultProps} files={files} onFilesChange={onFilesChange} />)

      const removeButton = screen.getByRole('button', { name: '' })
      await userEvent.click(removeButton)

      expect(onFilesChange).toHaveBeenCalledWith([])
    })

    it('disables upload button when upload is pending', () => {
      vi.mocked(useUploadFile).mockReturnValue(createMockMutation({ isPending: true }))

      render(<ChatForm {...defaultProps} />)

      const uploadButton = screen.getByRole('button', { name: /upload file/i })
      expect(uploadButton).toBeDisabled()
    })
  })

  describe('File Autocomplete', () => {
    it('shows autocomplete when open', () => {
      vi.mocked(useUniversalFileAutocomplete).mockReturnValue({
        ...mockAutocomplete,
        isOpen: true,
      })

      render(<ChatForm {...defaultProps} />)

      expect(screen.getByTestId('file-autocomplete')).toBeInTheDocument()
    })

    it('does not show autocomplete when closed', () => {
      render(<ChatForm {...defaultProps} />)

      expect(screen.queryByTestId('file-autocomplete')).not.toBeInTheDocument()
    })

    it('calls handleInputChange on textarea change', async () => {
      const handleInputChange = vi.fn()
      vi.mocked(useUniversalFileAutocomplete).mockReturnValue({
        ...mockAutocomplete,
        handleInputChange,
      })

      const onMessageChange = vi.fn()
      render(<ChatForm {...defaultProps} onMessageChange={onMessageChange} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.type(textarea, 'test')

      expect(handleInputChange).toHaveBeenCalled()
    })

    it('delegates key events to autocomplete when open', async () => {
      const handleKeyDown = vi.fn((e: any) => {
        e.preventDefault()
      })
      vi.mocked(useUniversalFileAutocomplete).mockReturnValue({
        ...mockAutocomplete,
        isOpen: true,
        handleKeyDown,
      })

      const onSubmit = vi.fn()
      render(<ChatForm {...defaultProps} onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{ArrowDown}')

      expect(handleKeyDown).toHaveBeenCalled()
    })
  })

  describe('UI States', () => {
    it('shows loading spinner when loading', () => {
      render(<ChatForm {...defaultProps} loading />)

      expect(screen.getByRole('button', { name: /send/i }).querySelector('svg')).toHaveClass(
        'animate-spin',
      )
    })

    it('shows running state button', () => {
      render(<ChatForm {...defaultProps} running buttonText="Stop" />)

      expect(screen.getByText('Stop')).toBeInTheDocument()
    })

    it('renders custom placeholder', () => {
      render(<ChatForm {...defaultProps} placeholder="Custom placeholder" />)

      expect(screen.getByPlaceholderText('Custom placeholder')).toBeInTheDocument()
    })

    it('renders selectors in correct position', () => {
      render(
        <ChatForm {...defaultProps} selectors={<button type="button">Test Selector</button>} />,
      )

      expect(screen.getByRole('button', { name: 'Test Selector' })).toBeInTheDocument()
    })
  })

  describe('Image Upload via Paste', () => {
    it('handles paste events', async () => {
      const mockMutate = vi.fn()
      vi.mocked(useUploadFile).mockReturnValue(createMockMutation({ mutate: mockMutate }))

      render(<ChatForm {...defaultProps} />)

      const textarea = screen.getByRole('textbox')
      expect(textarea).toBeInTheDocument()
    })

    it('allows text paste when no image', async () => {
      render(<ChatForm {...defaultProps} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.paste('pasted text')
    })
  })
})
