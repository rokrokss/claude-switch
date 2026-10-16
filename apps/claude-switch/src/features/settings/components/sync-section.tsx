import type { ICloudSync } from '@/lib/types'

import { formatDistanceToNow } from 'date-fns'
import { TriangleAlert } from 'lucide-react'

import { Switch } from '@/design'
import { useICloudSync } from '@/features/settings/api/use-icloud-sync'
import { useAppState } from '@/lib/app-state/use-app-state'

type Device = ICloudSync['devices'][number]

/**
 * The Sync section of Settings: iCloud sync of the usage history, off on a fresh install, and the
 * Macs whose files it reads while on.
 */
export function SyncSection() {
  const appState = useAppState()
  const status = useICloudSync()
  const on = appState.state.settings.sync.icloud

  function handleChange(next: boolean) {
    void appState.update({ settings: { sync: { icloud: next } } })
  }

  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">Sync</span>
        {on && status?.syncing ? <span className="text-[11px] text-muted-strong">Syncing…</span> : null}
      </div>
      <div className="rounded-xl border border-border bg-white py-1 dark:bg-cream-2">
        <div className="flex items-center gap-4 border-b border-border-soft px-4 py-3 last:border-b-0">
          <div className="flex-1">
            <span id="sync-icloud" className="block text-[13px] font-medium text-ink">
              Sync usage across Macs
            </span>
            <span id="sync-icloud-detail" className="mt-0.5 block text-[12px] leading-[1.4] text-muted">
              Adds what each account spent on your other Macs signed in to the same iCloud account. Turning it off
              deletes this Mac's file from iCloud.
            </span>
          </div>
          <Switch
            aria-labelledby="sync-icloud"
            aria-describedby="sync-icloud-detail"
            checked={on}
            onChange={handleChange}
          />
        </div>
        {on && status ? status.devices.map((device) => <DeviceRow key={device.id} device={device} />) : null}
      </div>
      <p className="mt-2 text-[11.5px] text-muted-strong">
        Each Mac writes only its own tokens and estimated cost, by day and model, under a one-way hash of each account.
        Never credentials, prompts or limits.
      </p>
      {on && status ? <SyncNotice status={status} /> : null}
    </section>
  )
}

/**
 * One Mac with a file: when it last wrote, and which of this Mac's profiles its usage counts for.
 */
function DeviceRow({ device }: { device: Device }) {
  const profiles = device.profiles.join(', ')
  let detail: string
  if (device.thisMac) {
    detail = profiles ? `Sharing ${profiles}` : 'Nothing to share yet'
  } else {
    detail = profiles ? `Adds to ${profiles}` : 'No account in common with this Mac'
  }
  return (
    <div className="flex items-center gap-4 border-b border-border-soft px-4 py-2.5 last:border-b-0">
      <div className="min-w-0 flex-1">
        <span className="block truncate text-[13px] text-ink">
          {device.name}
          {device.thisMac ? <span className="ml-1.5 text-[11.5px] text-muted-strong">This Mac</span> : null}
        </span>
        <span className="mt-0.5 block truncate text-[12px] text-muted">{detail}</span>
      </div>
      <span className="shrink-0 text-[11.5px] text-muted-strong">
        {formatDistanceToNow(new Date(device.updatedAt), { addSuffix: true })}
      </span>
    </div>
  )
}

/**
 * What keeps sync from working, if anything, and the files left out.
 */
function SyncNotice({ status }: { status: ICloudSync }) {
  if (status.problem === null) {
    return null
  }
  return (
    <div role="status" className="mt-2.5 flex gap-2.5 text-[12px] text-ink-soft">
      <TriangleAlert aria-hidden className="mt-px h-3.5 w-3.5 shrink-0 text-amber" strokeWidth={1.85} />
      <div className="min-w-0 flex-1">
        <span className="block">{status.problem}</span>
        {status.unreadable.map((file) => (
          <span key={file} className="mt-0.5 block truncate font-mono text-[11px] text-muted-strong">
            {file}
          </span>
        ))}
      </div>
    </div>
  )
}
