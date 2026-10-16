import { cn } from '@/design/lib/cn'

type SwitchProps = {
  checked: boolean
  disabled?: boolean
  /**
   * Name the switch with `aria-label`, or with `aria-labelledby` pointing at a visible label.
   */
  'aria-label'?: string
  'aria-labelledby'?: string
  'aria-describedby'?: string
  className?: string
  onChange: (next: boolean) => void
}

/**
 * An on/off switch for a setting that takes effect at once. Mint when on, like the checkbox of a
 * selected option.
 */
export function Switch({ checked, disabled, className, onChange, ...aria }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative inline-flex h-[18px] w-[30px] shrink-0 cursor-pointer items-center rounded-full border-[1.5px] outline-none transition-colors duration-(--duration-snap) ease-(--ease-natural)',
        'focus-visible:ring-2 focus-visible:ring-focus/40 focus-visible:ring-offset-1 focus-visible:ring-offset-cream',
        'disabled:cursor-not-allowed disabled:opacity-60',
        checked ? 'border-ink bg-mint' : 'border-border bg-cream',
        className,
      )}
      {...aria}
    >
      <span
        aria-hidden
        className={cn(
          'h-[10px] w-[10px] rounded-full transition-[transform,background-color] duration-(--duration-snap) ease-(--ease-natural)',
          checked ? 'translate-x-[14px] bg-accent-ink' : 'translate-x-[2px] bg-muted',
        )}
      />
    </button>
  )
}

export type { SwitchProps }
