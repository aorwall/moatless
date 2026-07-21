export interface SidebarTaskFilterValues {
  status: 'all' | 'open' | 'error' | 'closed'
  tag?: string
  createdBy?: string
  repositoryId?: string
  onlyFollowed: boolean
}

export const DEFAULT_SIDEBAR_FILTERS: SidebarTaskFilterValues = {
  status: 'open',
  createdBy: 'all',
  onlyFollowed: true,
}

export function hasActiveSidebarFilters(filters: SidebarTaskFilterValues): boolean {
  return (
    filters.status !== DEFAULT_SIDEBAR_FILTERS.status ||
    !!filters.tag ||
    (!!filters.createdBy && filters.createdBy !== 'all') ||
    !!filters.repositoryId ||
    filters.onlyFollowed !== DEFAULT_SIDEBAR_FILTERS.onlyFollowed
  )
}
