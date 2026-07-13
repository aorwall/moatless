import type { UiMessage } from '@moatless/api-client'
import { getAdapterIcon, getAdapterLabel } from './external-source-meta'

export function ExternalSourceChip({
  source,
}: {
  source: NonNullable<UiMessage['externalSource']>
}) {
  if (!source.externalLabel) {
    return null
  }
  return (
    <div className="flex items-center gap-1.5 mb-1 text-xs text-muted-foreground">
      <span className="text-blue-500/70">{getAdapterIcon(source.adapterKind)}</span>
      <span className="font-medium">via {getAdapterLabel(source.adapterKind)}</span>
      {source.sourceName && <span className="text-muted-foreground/70">{source.sourceName}</span>}
      <span
        className="ml-auto font-mono text-[10px] text-muted-foreground/70"
        title={source.externalId ?? source.externalLabel}
      >
        {source.externalLabel}
      </span>
    </div>
  )
}
