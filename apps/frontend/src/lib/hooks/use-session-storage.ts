import { useCallback, useState } from 'react'

/**
 * `sessionStorage` with React state synchronization — the tab-scoped
 * counterpart to `useLocalStorage`, for state that should reset on a fresh
 * visit (e.g. dismissed banners).
 *
 * No cross-tab `storage` listener: that event doesn't fire for sessionStorage.
 *
 * @param defaultValue - Value to use when the key is absent or unreadable
 */
export function useSessionStorage<T>(key: string, defaultValue: T): [T, (value: T) => void] {
  const [storedValue, setStoredValue] = useState<T>(() => {
    if (typeof window === 'undefined') {
      return defaultValue
    }

    try {
      const item = sessionStorage.getItem(key)
      if (item === null) {
        return defaultValue
      }

      // For strings, return as-is. For objects, parse JSON
      if (typeof defaultValue === 'string') {
        return item as T
      }

      return JSON.parse(item) as T
    } catch (_error) {
      return defaultValue
    }
  })

  const setValue = useCallback(
    (value: T) => {
      try {
        setStoredValue(value)

        if (typeof window !== 'undefined') {
          const valueToStore = typeof value === 'string' ? value : JSON.stringify(value)
          sessionStorage.setItem(key, valueToStore as string)
        }
      } catch (_error) {
        // sessionStorage.setItem may throw in private browsing mode
      }
    },
    [key],
  )

  return [storedValue, setValue]
}
