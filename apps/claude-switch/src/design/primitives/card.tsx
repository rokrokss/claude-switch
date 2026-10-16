import type { HTMLAttributes } from 'react'

import { cn } from '@/design/lib/cn'

type CardProps = HTMLAttributes<HTMLDivElement>

export function Card({ className, children, ...rest }: CardProps) {
  return (
    <div className={cn('glass rounded-xl border', className)} {...rest}>
      {children}
    </div>
  )
}
