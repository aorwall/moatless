import type { Preview } from '@storybook/react'
import '../src/lib/styles/globals.css'
import '../../../packages/diff-viewer/src/styles/diff-theme.css'

const preview: Preview = {
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
  },
}

export default preview
