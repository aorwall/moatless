import { createFileRoute } from '@tanstack/react-router'
import { FigmaClipboardTestPage } from '@/features/figma/pages/figma-clipboard-test'

export const Route = createFileRoute('/figma-test')({
  component: FigmaClipboardTestPage,
})
