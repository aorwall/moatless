import { type Dispatch, type SetStateAction, useEffect, useState } from 'react'

/**
 * A hook that provides localStorage functionality with React state synchronization.
 *
 * @param defaultValue - The default value to use if no value exists in localStorage
 */
export function useLocalStorage<T>(key: string, defaultValue: T): [T, Dispatch<SetStateAction<T>>] {
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

  useEffect(() => {
    try {
      if (typeof window !== 'undefined') {
        const valueToStore =
          typeof storedValue === 'string' ? storedValue : JSON.stringify(storedValue)
        localStorage.setItem(key, valueToStore)
      }
    } catch (_error) {
      // localStorage.setItem may throw in private browsing mode
    }
  }, [key, storedValue])

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

  return [storedValue, setStoredValue]
}
