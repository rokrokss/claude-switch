import type { ICloudSync } from '@/lib/types'

import { useEffect } from 'react'

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'

import { getICloudSync } from '@/lib/commands'
import { queryKeys } from '@/lib/query/keys'

/**
 * iCloud sync's status, kept current by the backend's `icloud-sync` events;
 * undefined until the first answer.
 */
export function useICloudSync(): ICloudSync | undefined {
  const queryClient = useQueryClient()
  const { data } = useQuery({ queryKey: queryKeys.icloudSync, queryFn: getICloudSync })

  useEffect(() => {
    const unlistenPromise = listen<ICloudSync>('icloud-sync', (event) => {
      queryClient.setQueryData(queryKeys.icloudSync, event.payload)
    })
    return () => {
      void unlistenPromise.then((unlisten) => {
        unlisten()
      })
    }
  }, [queryClient])

  return data
}
