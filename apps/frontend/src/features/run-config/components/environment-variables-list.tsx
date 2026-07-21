import type { ClipboardEvent } from 'react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'

function parseEnvContent(text: string): Record<string, string> {
  const result: Record<string, string> = {}
  for (const line of text.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) {
      continue
    }
    const eqIndex = trimmed.indexOf('=')
    if (eqIndex === -1) {
      continue
    }
    const key = trimmed.slice(0, eqIndex).trim()
    if (!key) {
      continue
    }
    let value = trimmed.slice(eqIndex + 1).trim()
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1)
    }
    result[key] = value
  }
  return result
}

interface EnvironmentVariablesListProps {
  variables: Record<string, string>
  onChange: (vars: Record<string, string>) => void
  disabled?: boolean
}

export function EnvironmentVariablesList({
  variables,
  onChange,
  disabled = false,
}: EnvironmentVariablesListProps) {
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')
  const [touched, setTouched] = useState(false)

  const hasPartialInput = (newKey !== '' || newValue !== '') && !(newKey !== '' && newValue !== '')
  const showKeyError = touched && newValue !== '' && newKey === ''
  const showValueError = touched && newKey !== '' && newValue === ''

  const handlePaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const text = e.clipboardData.getData('text/plain')
    const parsed = parseEnvContent(text)
    const entries = Object.entries(parsed)
    if (entries.length >= 2) {
      e.preventDefault()
      onChange({ ...variables, ...parsed })
      setNewKey('')
      setNewValue('')
      setTouched(false)
    } else if (entries.length === 1) {
      const entry = entries[0]
      if (!entry) {
        return
      }
      const [key, value] = entry
      e.preventDefault()
      setNewKey(key)
      setNewValue(value)
    }
  }

  const commitNew = () => {
    if (newKey && newValue) {
      onChange({ ...variables, [newKey]: newValue })
      setNewKey('')
      setNewValue('')
      setTouched(false)
    } else if (hasPartialInput) {
      setTouched(true)
    }
  }

  const handleRemove = (key: string) => {
    onChange(Object.fromEntries(Object.entries(variables).filter(([entryKey]) => entryKey !== key)))
  }

  const handleKeyChange = (oldKey: string, newKeyName: string) => {
    const entries = Object.entries(variables).map(([k, v]) =>
      k === oldKey ? [newKeyName, v] : [k, v],
    )
    onChange(Object.fromEntries(entries))
  }

  const handleValueChange = (key: string, newVal: string) => {
    onChange({ ...variables, [key]: newVal })
  }

  return (
    <div className="space-y-3">
      {Object.entries(variables).map(([key, value]) => (
        <div key={key} className="flex items-center gap-2">
          <Input
            value={key}
            onChange={(e) => handleKeyChange(key, e.target.value)}
            className="flex-1"
            disabled={disabled}
          />
          <Input
            value={value}
            onChange={(e) => handleValueChange(key, e.target.value)}
            className="flex-1"
            disabled={disabled}
          />
          {!disabled && (
            <Button type="button" variant="outline" size="sm" onClick={() => handleRemove(key)}>
              Remove
            </Button>
          )}
        </div>
      ))}
      {!disabled && (
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Input
              placeholder="New variable name"
              value={newKey}
              onChange={(e) => setNewKey(e.target.value)}
              onPaste={handlePaste}
              onBlur={commitNew}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  commitNew()
                }
              }}
              className={`flex-1 ${showKeyError ? 'border-destructive' : ''}`}
            />
            <Input
              placeholder="Value"
              value={newValue}
              onChange={(e) => setNewValue(e.target.value)}
              onBlur={commitNew}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  commitNew()
                }
              }}
              className={`flex-1 ${showValueError ? 'border-destructive' : ''}`}
            />
            <div className="w-[73px]" />
          </div>
          {touched && hasPartialInput && (
            <p className="text-xs text-destructive">Both name and value are required</p>
          )}
        </div>
      )}
    </div>
  )
}
