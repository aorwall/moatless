import { QueryClient } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

import { queryKeys } from '@/lib/api/query-keys'
import { invalidateGitHubInstallationCaches } from '../use-admin-github-connection'

describe('invalidateGitHubInstallationCaches', () => {
  it('invalidates installation, repository/branch/access, setup, and task setup caches', async () => {
    const queryClient = new QueryClient()
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue()

    await invalidateGitHubInstallationCaches(queryClient, 'team-1')

    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.admin.github() })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.repositories.all() })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.auth.session() })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.tasks.all })
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.admin.teamGithubInstallation('team-1'),
    })
  })
})
