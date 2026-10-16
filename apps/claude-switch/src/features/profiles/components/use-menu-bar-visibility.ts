import type { AppId } from '@/lib/app-registry'

import { appSpecs } from '@/lib/app-registry'
import { useAppState } from '@/lib/app-state/use-app-state'
import { extractErrorMessage } from '@/lib/extract-error-message'

type MenuBarVisibility = {
  /**
   * Whether the entry's usage shows in the menu bar's strip.
   */
  shownInMenuBar: boolean
  /**
   * Shows or hides it. Absent when there is nothing to show: the menu bar reads
   * usage through the CLI's sign-in, so an entry without its CLI has none.
   */
  onShownInMenuBarChange?: (shown: boolean) => void
}

/**
 * The "Show in menu bar" option of entry `id` (a managed profile, or a stock
 * install's `default:<app>`), for its "…" menu. A failed save is reported
 * through `onError`.
 */
export function useMenuBarVisibility(
  id: string,
  app: AppId,
  cli: boolean,
  onError: (message: string) => void,
): MenuBarVisibility {
  const appState = useAppState()
  const offered = cli && appSpecs[app].hasUsage
  return {
    shownInMenuBar: !appState.state.hiddenFromMenuBar.includes(id),
    onShownInMenuBarChange: offered
      ? (shown) => {
          appState.update({ menuBar: { id, shown } }).catch((caught: unknown) => {
            onError(extractErrorMessage(caught))
          })
        }
      : undefined,
  }
}
