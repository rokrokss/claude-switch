import type { Surfaces } from '@/lib/types'

import { Monitor, Terminal } from 'lucide-react'

import { cn } from '@/design'

type Props = {
  surfaces: Surfaces
  /**
   * Whether the row's app has a desktop app. Without one (Gemini) there is
   * no monitor to show as on or off.
   */
  hasDesktop: boolean
}

/**
 * The two surface glyphs (desktop monitor + terminal) sit at the right edge
 * of a profile row. Surfaces toggled off render at 22% opacity so the row
 * still reads as a single profile but the missing capability is obvious.
 */
export function SidebarSurfaceIcons({ surfaces, hasDesktop }: Props) {
  return (
    <span className="flex items-center gap-1 text-muted">
      {hasDesktop ? (
        <Monitor
          aria-label={surfaces.gui ? 'Desktop app enabled' : 'Desktop app disabled'}
          strokeWidth={1.75}
          className={cn('h-3 w-3', surfaces.gui ? 'opacity-100' : 'opacity-[0.22]')}
        />
      ) : null}
      <Terminal
        aria-label={surfaces.cli ? 'CLI enabled' : 'CLI disabled'}
        strokeWidth={1.75}
        className={cn('h-3 w-3', surfaces.cli ? 'opacity-100' : 'opacity-[0.22]')}
      />
    </span>
  )
}
