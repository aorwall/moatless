import { useCallback, useState } from 'react'
import type { z } from 'zod'

export interface UseZodFormOptions<T extends z.ZodType> {
  schema: T
  onSubmit?: (data: z.infer<T>) => void | Promise<void>
}

export interface UseZodFormReturn<T extends z.ZodType> {
  errors: Partial<Record<keyof z.infer<T>, string>>
  validate: (data: unknown) => z.SafeParseReturnType<z.infer<T>, z.infer<T>>
  handleSubmit: (data: unknown) => Promise<boolean>
  clearErrors: () => void
  setFieldError: (field: keyof z.infer<T>, error: string) => void
}

/**
 * Custom hook for form validation using Zod schemas
 * Provides validation, error handling, and form submission utilities
 */
export function useZodForm<T extends z.ZodType>({
  schema,
  onSubmit,
}: UseZodFormOptions<T>): UseZodFormReturn<T> {
  const [errors, setErrors] = useState<Partial<Record<keyof z.infer<T>, string>>>({})

  const validate = useCallback(
    (data: unknown) => {
      const result = schema.safeParse(data)

      if (!result.success) {
        const fieldErrors: Partial<Record<keyof z.infer<T>, string>> = {}

        // Convert Zod errors to field-specific errors
        for (const error of result.error.errors) {
          const path = error.path[0] as keyof z.infer<T>
          if (path && !fieldErrors[path]) {
            fieldErrors[path] = error.message
          }
        }

        setErrors(fieldErrors)
      } else {
        setErrors({})
      }

      return result
    },
    [schema],
  )

  const handleSubmit = useCallback(
    async (data: unknown): Promise<boolean> => {
      const result = validate(data)

      if (result.success && onSubmit) {
        try {
          await onSubmit(result.data)
          return true
        } catch (error) {
          console.error('Form submission error:', error)
          return false
        }
      }

      return result.success
    },
    [validate, onSubmit],
  )

  const clearErrors = useCallback(() => {
    setErrors({})
  }, [])

  const setFieldError = useCallback((field: keyof z.infer<T>, error: string) => {
    setErrors((prev) => ({ ...prev, [field]: error }))
  }, [])

  return {
    errors,
    validate,
    handleSubmit,
    clearErrors,
    setFieldError,
  }
}