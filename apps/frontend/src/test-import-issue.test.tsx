import { QueryClient } from '@tanstack/react-query'
import { describe, expect, it } from 'vitest'

describe('Import Test', () => {
  it('should import React Query without issues', () => {
    const queryClient = new QueryClient()
    expect(queryClient).toBeDefined()
  })

  it('should render JSX without issues', () => {
    const element = <div>Test</div>
    expect(element).toBeDefined()
  })
})
