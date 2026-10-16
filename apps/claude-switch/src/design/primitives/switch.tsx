import { cn } from '../lib/cn'

type SwitchProps = {
  checked: boolean
  onChange: (checked: boolean) => void
  /**
   * Names the setting for assistive tech when no visible label is tied to it.
   */
  ariaLabel: string
  disabled?: boolean
}

/**
 * An on/off switch for a setting that takes effect at once. For a choice
 * kept in a form until it is submitted, use a checkbox row instead.
 */
export function Switch({ checked, onChange, ariaLabel, disabled = false }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative inline-flex h-[18px] w-[30px] shrink-0 cursor-pointer items-center rounded-full border-[1.5px] transition-colors duration-(--duration-snap) ease-(--ease-natural)',
        'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus/40',
        'disabled:cursor-not-allowed disabled:opacity-50',
        checked ? 'border-ink bg-mint' : 'border-border bg-cream-3',
      )}
    >
      <span
        aria-hidden
        className={cn(
          'block h-3 w-3 rounded-full bg-cream shadow-sm ring-1 ring-ink/15 transition-transform duration-(--duration-snap) ease-(--ease-natural)',
          checked ? 'translate-x-[13px]' : 'translate-x-[2px]',
        )}
      />
    </button>
  )
}
