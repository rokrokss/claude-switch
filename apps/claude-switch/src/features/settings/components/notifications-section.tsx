import type { NotificationPermission, Settings } from '@/lib/types'

import { TriangleAlert } from 'lucide-react'

import { Button, Switch } from '@/design'
import { useNotificationPermission } from '@/features/settings/api/use-notification-permission'
import { useAppState } from '@/lib/app-state/use-app-state'
import { openNotificationSettings } from '@/lib/commands'

type Triggers = Settings['notifications']

const triggers: ReadonlyArray<{ key: keyof Triggers; title: string; description: string }> = [
  {
    key: 'almostOut',
    title: 'Almost out',
    description: 'When less than 10% of a limit is left.',
  },
  {
    key: 'cuttingItClose',
    title: 'Cutting it close',
    description: 'When a limit is on pace to end with little to spare.',
  },
  {
    key: 'willRunOut',
    title: 'Will run out',
    description: 'When a limit is on pace to run out before it resets.',
  },
]

/**
 * The Notifications section of Settings: a switch per trigger, all off on a fresh install.
 *
 * Turning the first one on asks macOS for permission. While any is on and macOS hasn't allowed
 * notifications, a notice under the switches says so, with the way to fix it.
 */
export function NotificationsSection() {
  const appState = useAppState()
  const notifications = appState.state.settings.notifications
  const anyOn = triggers.some((trigger) => notifications[trigger.key])
  const permission = useNotificationPermission(anyOn)

  function handleChange(key: keyof Triggers, next: boolean) {
    void appState.update({ settings: { notifications: { ...notifications, [key]: next } } })
    if (next && !anyOn) {
      void permission.request()
    }
  }

  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">
          Notifications
        </span>
      </div>
      <div className="glass rounded-xl border border-border bg-white py-1 dark:bg-cream-2">
        {triggers.map((trigger) => (
          <div
            key={trigger.key}
            className="flex items-center gap-4 border-b border-border-soft px-4 py-3 last:border-b-0"
          >
            <div className="flex-1">
              <span id={`notify-${trigger.key}`} className="block text-[13px] font-medium text-ink">
                {trigger.title}
              </span>
              <span id={`notify-${trigger.key}-detail`} className="mt-0.5 block text-[12px] leading-[1.4] text-muted">
                {trigger.description}
              </span>
            </div>
            <Switch
              aria-labelledby={`notify-${trigger.key}`}
              aria-describedby={`notify-${trigger.key}-detail`}
              checked={notifications[trigger.key]}
              onChange={(next) => handleChange(trigger.key, next)}
            />
          </div>
        ))}
      </div>
      <p className="mt-2 text-[11.5px] text-muted-strong">
        Every profile's limits are watched. Each alert comes once, until its limit resets.
      </p>
      <PermissionNotice permission={permission.permission} onAllow={permission.request} />
    </section>
  )
}

type PermissionNoticeProps = {
  permission: NotificationPermission | null
  onAllow: () => Promise<void>
}

/**
 * What stops the notifications from showing, if anything: a refusal, which only System Settings can
 * undo, or a question macOS hasn't asked yet.
 */
function PermissionNotice({ permission, onAllow }: PermissionNoticeProps) {
  if (permission !== 'denied' && permission !== 'notDetermined') {
    return null
  }
  const denied = permission === 'denied'
  return (
    <div role="status" className="mt-2.5 flex items-center gap-2.5 text-[12px] text-ink-soft">
      <TriangleAlert aria-hidden className="h-3.5 w-3.5 shrink-0 text-amber" strokeWidth={1.85} />
      <span className="flex-1">
        {denied
          ? 'macOS is set to block notifications from claude-switch.'
          : 'claude-switch needs your permission to post notifications.'}
      </span>
      <Button size="sm" variant="secondary" onClick={() => void (denied ? openNotificationSettings() : onAllow())}>
        {denied ? 'Open System Settings' : 'Allow notifications'}
      </Button>
    </div>
  )
}
