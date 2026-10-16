import type { AppId } from '@/lib/types'

import { Button, Dialog, Kbd } from '@/design'
import { appSpecs } from '@/lib/app-registry'

type Props = {
  open: boolean
  app: AppId
  onClose: () => void
}

/**
 * What a signed clone involves, as points. Each follows from how it is done:
 * the profile runs from an untouched copy of the app, which macOS sees as the
 * original, started without the profile when macOS rather than claude-switch
 * starts it.
 */
function points(displayName: string): Array<string> {
  return [
    `This profile runs from its own copy of ${displayName}, kept with the profile's data. The copy shares the original's disk space until it updates itself.`,
    `The copy keeps ${displayName}'s own signature, so sign-in, permissions like the microphone and camera, passkeys and Cowork's folders work as they do in the original. The only addition is a Finder icon — the app's own, with a badge in the profile's color — which is what the Dock shows.`,
    `The copy updates itself. When ${displayName} in Applications is newer, opening the profile moves the copy forward first; it is never taken back to an older version, and never changed while it runs.`,
    'When macOS starts the copy on its own — after an update, or from its Dock tile — it would open on your default account. claude-switch quits it within seconds and opens the profile instead, or, if it has been running a while, asks first.',
    'Opening the profile, and watching for that, is done by claude-switch: it installs a small background item while any profile uses a signed clone.',
  ]
}

/**
 * Explains the signed clone option. It asks nothing: turning the option on
 * costs nothing the user would notice, so there is no question to answer, just
 * this to read. It opens over the form the option is in.
 */
export function SignedCloneDialog({ open, app, onClose }: Props) {
  const { displayName } = appSpecs[app]

  return (
    <Dialog
      stacked
      open={open}
      title="A signed clone"
      description={`Gives this profile its own Dock icon while keeping ${displayName} as its developer signed it. Turning it off deletes the copy and goes back to the regular app. Your data stays where it is either way.`}
      className="w-[min(640px,calc(100%-64px))]"
      onClose={onClose}
      onSubmit={onClose}
      foot={
        <Button variant="primary" size="sm" trailingKbd={<Kbd variant="onAccent">⏎</Kbd>} onClick={onClose}>
          Close
        </Button>
      }
    >
      <ul className="space-y-2">
        {points(displayName).map((point) => (
          <li
            key={point}
            className="relative pl-3.5 text-body text-muted before:absolute before:left-0 before:content-['•']"
          >
            {point}
          </li>
        ))}
      </ul>
    </Dialog>
  )
}
