import type { AdapterKind, Loop } from '@moatless/api-client'
import { getAdapterLabel } from '@/features/integrations/utils/adapter-display'

export { getAdapterLabel } from '@/features/integrations/utils/adapter-display'

export type LoopSource = Loop['source']

export function isAdapterEventSource(
  source: LoopSource,
): source is Extract<LoopSource, { adapterKind: AdapterKind }> {
  return 'adapterKind' in source
}

export function isScheduleSource(
  source: LoopSource,
): source is Extract<LoopSource, { cronExpression: string }> {
  return 'cronExpression' in source
}

/**
 * Where a Loop listens, as one line: the provider it fires on and the subject it
 * watches there. The name says what the Loop is for, so it never has to repeat
 * the provider.
 */
export function getSourceLabel(loop: Loop): string {
  const source = loop.source
  if (isAdapterEventSource(source)) {
    return `${getAdapterLabel(source.adapterKind)} · ${source.sourceName || source.sourceMatcher}`
  }
  if (isScheduleSource(source)) {
    return `Schedule · ${source.cronExpression}`
  }
  return 'Manual'
}
