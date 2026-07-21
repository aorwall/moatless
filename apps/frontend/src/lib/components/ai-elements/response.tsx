'use client'

import React, { type ComponentProps } from 'react'
import { Streamdown } from 'streamdown'
import { cn } from '@/lib/utils.ts'

type ResponseProps = ComponentProps<typeof Streamdown>

// `React.memo` (not a named `memo` import) dodges a Biome 2.5.4 module-resolver
// panic when the type-aware nursery rules resolve React's declarations here.
export const Response = React.memo(
  ({ className, ...props }: ResponseProps) => (
    <Streamdown
      className={cn('size-full [&>*:first-child]:mt-0 [&>*:last-child]:mb-0', className)}
      {...props}
    />
  ),
  (prevProps, nextProps) => prevProps.children === nextProps.children,
)

Response.displayName = 'Response'
