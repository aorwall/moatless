/**
 * Filter (and optionally sort) a list for a search sidebar.
 *
 * `getSearchText` returns the fields to match the (case-insensitive) search term
 * against; nullish entries are ignored. When `search` is empty, all items pass.
 *
 * Note: `getSearchText` and `sortBy` should be referentially stable (module-level
 * or `useCallback`) — otherwise the memo recomputes every render. That's harmless
 * for the small lists in the admin area, but keep it in mind for larger lists.
 */
export function useListFilter<T>(
  items: T[] | undefined,
  search: string,
  getSearchText: (item: T) => Array<string | null | undefined>,
  sortBy?: (a: T, b: T) => number,
): T[] {
  return (() => {
    const all = items ?? []
    const term = search.trim().toLowerCase()
    const filtered = term
      ? all.filter((item) =>
          getSearchText(item)
            .filter((value): value is string => Boolean(value))
            .some((value) => value.toLowerCase().includes(term)),
        )
      : all
    return sortBy ? filtered.toSorted(sortBy) : filtered
  })()
}
