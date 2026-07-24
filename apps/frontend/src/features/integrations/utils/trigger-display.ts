import type { AdapterKind, Loop } from '@moatless/api-client'

type TriggerSource = Loop['source']

const ADAPTER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

export function isAdapterEventSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { adapterKind: AdapterKind }> {
  return 'adapterKind' in source
}

export function isScheduleSource(
  source: TriggerSource,
): source is Extract<TriggerSource, { cronExpression: string }> {
  return 'cronExpression' in source
}

export function getAdapterLabel(adapter: string): string {
  return ADAPTER_LABELS[adapter] ?? adapter
}

export function getSourceLabel(trigger: Loop): string {
  if (isAdapterEventSource(trigger.source)) {
    return trigger.source.sourceName || trigger.source.sourceMatcher
  }
  if (isScheduleSource(trigger.source)) {
    return trigger.name || trigger.source.cronExpression
  }
  return trigger.name || trigger.id
}

export function getTriggerKindLabel(trigger: Loop): string {
  return isAdapterEventSource(trigger.source)
    ? getAdapterLabel(trigger.source.adapterKind)
    : 'Schedule'
}
