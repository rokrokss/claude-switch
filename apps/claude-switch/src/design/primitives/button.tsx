import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react'

import { cn } from '@/design/lib/cn'

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'
type ButtonSize = 'sm' | 'md'

type ButtonProps = {
  variant?: ButtonVariant
  size?: ButtonSize
  leadingIcon?: ReactNode
  trailingIcon?: ReactNode
  trailingKbd?: ReactNode
  ref?: Ref<HTMLButtonElement>
  children?: ReactNode
} & ButtonHTMLAttributes<HTMLButtonElement>

const baseClasses =
  'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap font-medium leading-none cursor-pointer select-none outline-none transition-[background-color,border-color,color,filter,box-shadow,transform] duration-(--duration-snap) ease-(--ease-natural) focus-visible:ring-2 focus-visible:ring-focus/40 focus-visible:ring-offset-1 focus-visible:ring-offset-cream disabled:cursor-not-allowed disabled:opacity-60'

const variantClasses: Record<ButtonVariant, string> = {
  primary:
    'text-accent-ink border-[1.5px] border-white/60 dark:border-ink bg-mint shadow-glass-button dark:shadow-button-primary hover:not-disabled:bg-mint-hover active:not-disabled:translate-x-px active:not-disabled:translate-y-px active:not-disabled:shadow-none',
  secondary:
    'text-ink-soft bg-cream border border-border hover:not-disabled:bg-white hover:not-disabled:border-border-strong hover:not-disabled:text-ink dark:hover:not-disabled:bg-cream-3 shadow-[0_1px_0_rgba(40,30,20,0.03)]',
  ghost: 'text-muted bg-transparent border-0 hover:not-disabled:bg-cream-2 hover:not-disabled:text-ink',
  danger:
    'text-red bg-cream border border-border hover:not-disabled:border-red/45 hover:not-disabled:bg-white dark:hover:not-disabled:bg-cream-3',
}

const sizeClasses: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 text-[12px] rounded-md',
  md: 'h-8 px-3 text-[12.5px] rounded-md',
}

export function Button({
  className,
  variant = 'secondary',
  size = 'md',
  leadingIcon,
  trailingIcon,
  trailingKbd,
  ref,
  children,
  type,
  ...rest
}: ButtonProps) {
  return (
    <button
      ref={ref}
      type={type ?? 'button'}
      data-variant={variant}
      data-size={size}
      className={cn(baseClasses, variantClasses[variant], sizeClasses[size], className)}
      {...rest}
    >
      {leadingIcon ? (
        <span
          aria-hidden
          className="inline-flex h-[var(--size-icon-sm)] w-[var(--size-icon-sm)] items-center justify-center"
        >
          {leadingIcon}
        </span>
      ) : null}
      {children !== undefined ? <span>{children}</span> : null}
      {trailingIcon ? (
        <span
          aria-hidden
          className="inline-flex h-[var(--size-icon-sm)] w-[var(--size-icon-sm)] items-center justify-center"
        >
          {trailingIcon}
        </span>
      ) : null}
      {trailingKbd ? <span className="ml-0.5 inline-flex items-center">{trailingKbd}</span> : null}
    </button>
  )
}

export type { ButtonProps, ButtonSize, ButtonVariant }
