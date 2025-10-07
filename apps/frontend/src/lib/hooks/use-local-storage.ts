import { useCallback, useEffect, useState } from 'react'

/**
 * A hook that provides localStorage functionality with React state synchronization.
 *
 * @param key - The localStorage key
 * @param defaultValue - The default value to use if no value exists in localStorage
 * @returns A tuple containing the current value and a setter function
 */
export function useLocalStorage<T>(key: string, defaultValue: T): [T, (value: T) => void] {
  // Initialize state with value from localStorage or default
  const [storedValue, setStoredValue] = useState<T>(() => {
    // SSR safety check
    if (typeof window === 'undefined') {
      return defaultValue
    }

    try {
      const item = localStorage.getItem(key)
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

  // Update localStorage when state changes
  const setValue = useCallback(
    (value: T) => {
      try {
        setStoredValue(value)

        if (typeof window !== 'undefined') {
          // For strings, store as-is. For objects, stringify
          const valueToStore = typeof value === 'string' ? value : JSON.stringify(value)
          localStorage.setItem(key, valueToStore as string)
        }
      } catch (_error) {
        // localStorage.setItem may throw in private browsing mode
      }
    },
    [key],
  )

  // Listen for changes to localStorage from other tabs/windows
  useEffect(() => {
    if (typeof window === 'undefined') {
      return
    }

    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === key && e.newValue !== null) {
        try {
          const newValue =
            typeof defaultValue === 'string' ? (e.newValue as T) : (JSON.parse(e.newValue) as T)
          setStoredValue(newValue)
        } catch (_error) {
          // JSON.parse may fail for malformed storage values
        }
      }
    }

    window.addEventListener('storage', handleStorageChange)
    return () => window.removeEventListener('storage', handleStorageChange)
  }, [key, defaultValue])

  return [storedValue, setValue]
}
