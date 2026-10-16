import type { ReactNode } from 'react'

/**
 * A Settings section: its small-caps title, then a card of rows.
 */
export function SettingsSection({
  title,
  action,
  children,
}: {
  title: string
  /**
   * A control beside the title, e.g. Undo.
   */
  action?: ReactNode
  children: ReactNode
}) {
  return (
    <section className="mb-8">
      <div className="mb-2.5 flex min-h-7 items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">{title}</span>
        {action}
      </div>
      <div className="glass rounded-xl border">{children}</div>
    </section>
  )
}

type SettingRowProps = {
  title: string
  description: string
  /**
   * The control that changes it, on the right.
   */
  control: ReactNode
  /**
   * A line under the description, for a state the user should know of.
   */
  note?: string | null
}

/**
 * One setting: what it is and what it does on the left, its control on the right.
 */
export function SettingRow({ title, description, control, note }: SettingRowProps) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-border-soft px-4 py-3 last:border-b-0">
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-ink">{title}</div>
        <div className="mt-0.5 text-[12px] leading-[1.4] text-muted">{description}</div>
        {note ? <div className="mt-1 text-[11.5px] text-amber">{note}</div> : null}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  )
}
