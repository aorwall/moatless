import type { SVGProps } from 'react'

/** Minimal Linear glyph — lucide has no Linear icon. */
export function LinearIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 100 100" fill="currentColor" aria-hidden="true" {...props}>
      <path d="M1.2 61.5a1 1 0 0 1 1.7-.5l36 36a1 1 0 0 1-.5 1.7A50 50 0 0 1 1.2 61.5ZM.1 47.6a1 1 0 0 0 .3.8l51.2 51.2a1 1 0 0 0 .8.3 50.2 50.2 0 0 0 9-1.6 1 1 0 0 0 .5-1.7L3.4 38.1a1 1 0 0 0-1.7.5 50.2 50.2 0 0 0-1.6 9ZM6.3 25.9a1 1 0 0 0 .2 1.1l66.5 66.5a1 1 0 0 0 1.1.2c2.4-1.1 4.7-2.4 6.8-3.9a1 1 0 0 0 .1-1.5L11.7 18.9a1 1 0 0 0-1.5.1c-1.5 2.1-2.8 4.4-3.9 6.9ZM18.4 11.3a1 1 0 0 1-.1-1.4A50 50 0 0 1 90.1 81.7a1 1 0 0 1-1.4-.1L18.4 11.3Z" />
    </svg>
  )
}
