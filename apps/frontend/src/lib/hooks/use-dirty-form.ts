import { useMemo, useState } from 'react'

function shallowEqual<T extends Record<string, unknown>>(a: T, b: T): boolean {
  const keys = Object.keys(a)
  if (keys.length !== Object.keys(b).length) {
    return false
  }
  return keys.every((key) => a[key] === b[key])
}

/**
 * Local form state with dirty tracking against an initial snapshot.
 *
 * `initial` must be referentially stable per entity — mount the form `key`ed by
 * the entity id (as the admin detail forms do). This hook intentionally does not
 * resync on background refetch, matching the existing behavior.
 */
export function useDirtyForm<T extends Record<string, unknown>>(initial: T) {
  const [values, setValues] = useState<T>(initial)

  const isDirty = useMemo(() => !shallowEqual(values, initial), [values, initial])

  const setField = <K extends keyof T>(key: K, value: T[K]) => {
    setValues((current) => ({ ...current, [key]: value }))
  }

  const reset = () => setValues(initial)

  return { values, setField, isDirty, reset }
}
