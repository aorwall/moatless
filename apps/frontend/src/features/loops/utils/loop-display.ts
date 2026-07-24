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

export function getSourceLabel(loop: Loop): string {
  if (isAdapterEventSource(loop.source)) {
    return loop.source.sourceName || loop.source.sourceMatcher
  }
  if (isScheduleSource(loop.source)) {
    return loop.name || loop.source.cronExpression
  }
  return loop.name || loop.id
}

export function getLoopKindLabel(loop: Loop): string {
  return isAdapterEventSource(loop.source) ? getAdapterLabel(loop.source.adapterKind) : 'Schedule'
}
