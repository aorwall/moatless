import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'

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
    const { [key]: _, ...rest } = variables
    onChange(rest)
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
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => handleRemove(key)}
            disabled={disabled}
          >
            Remove
          </Button>
        </div>
      ))}
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <Input
            placeholder="New variable name"
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            onBlur={commitNew}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                commitNew()
              }
            }}
            className={`flex-1 ${showKeyError ? 'border-destructive' : ''}`}
            disabled={disabled}
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
            disabled={disabled}
          />
          <div className="w-[73px]" />
        </div>
        {touched && hasPartialInput && (
          <p className="text-xs text-destructive">Both name and value are required</p>
        )}
      </div>
    </div>
  )
}
