import type { NotificationPermission } from '@/lib/types'

import { useEffect } from 'react'

import { useQuery, useQueryClient } from '@tanstack/react-query'

import { notificationPermission, requestNotificationPermission } from '@/lib/commands'
import { queryKeys } from '@/lib/query/keys'

type UseNotificationPermissionResult = {
  /**
   * `null` until read, and while not `enabled`.
   */
  permission: NotificationPermission | null
  /**
   * Asks macOS, if the user hasn't answered yet, and records the answer.
   */
  request: () => Promise<void>
}

/**
 * Whether macOS lets claude-switch post notifications, read only while `enabled` (a trigger is on).
 * Read again whenever the window regains focus: the user may be back from changing it in System
 * Settings.
 */
export function useNotificationPermission(enabled: boolean): UseNotificationPermissionResult {
  const queryClient = useQueryClient()
  const { data } = useQuery({
    queryKey: queryKeys.notificationPermission,
    queryFn: notificationPermission,
    enabled,
  })

  useEffect(() => {
    function reread() {
      void queryClient.invalidateQueries({ queryKey: queryKeys.notificationPermission })
    }
    window.addEventListener('focus', reread)
    return () => window.removeEventListener('focus', reread)
  }, [queryClient])

  return {
    permission: enabled ? (data ?? null) : null,
    request: async () => {
      queryClient.setQueryData(queryKeys.notificationPermission, await requestNotificationPermission())
    },
  }
}
