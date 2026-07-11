import { createLazyFileRoute } from '@tanstack/react-router'
import { FigmaClipboardTestPage } from '@/features/figma/pages/figma-clipboard-test'

export const Route = createLazyFileRoute('/figma-test')({
  component: FigmaClipboardTestPage,
})
