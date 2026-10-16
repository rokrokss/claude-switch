import type { CommandLine } from '@/lib/types'

import { useMutation, useQueryClient, useSuspenseQuery } from '@tanstack/react-query'

import { checkCommandLine, installCommandLine, uninstallCommandLine } from '@/lib/commands'
import { queryKeys } from '@/lib/query/keys'

type UseCommandLineResult = {
  commandLine: CommandLine
  install: () => Promise<CommandLine>
  uninstall: () => Promise<CommandLine>
  busy: boolean
}

/**
 * The `claude-switch` command in `~/.local/bin`: whether it is installed, and
 * installing or removing it.
 */
export function useCommandLine(): UseCommandLineResult {
  const queryClient = useQueryClient()
  const { data } = useSuspenseQuery({
    queryKey: queryKeys.commandLine,
    queryFn: checkCommandLine,
  })

  const mutation = useMutation({
    mutationFn: (install: boolean) => (install ? installCommandLine() : uninstallCommandLine()),
    onSuccess: (next) => {
      queryClient.setQueryData(queryKeys.commandLine, next)
    },
    onError: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.commandLine })
    },
  })

  return {
    commandLine: data,
    install: () => mutation.mutateAsync(true),
    uninstall: () => mutation.mutateAsync(false),
    busy: mutation.isPending,
  }
}
