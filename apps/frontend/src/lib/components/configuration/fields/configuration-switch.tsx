import { Switch } from '@/lib/components/ui/switch'

interface ConfigurationSwitchProps {
  checked?: boolean
  onCheckedChange?: (checked: boolean) => void
  disabled?: boolean
}

export function ConfigurationSwitch(props: ConfigurationSwitchProps) {
  return <Switch {...props} />
}
