import { useEffect, useState } from 'react'

import { X } from 'lucide-react'

import { Button } from '@/design'

import { acceleratorFrom, formatAccelerator } from '../lib/shortcut'

type Props = {
  value: string | null
  onChange: (value: string | null) => void
}

/**
 * Records a shortcut: click, then press it. Esc gives up; the cross clears it.
 */
export function ShortcutRecorder({ value, onChange }: Props) {
  const [recording, setRecording] = useState(false)

  useEffect(() => {
    if (!recording) {
      return
    }
    function onKey(event: KeyboardEvent) {
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Escape') {
        setRecording(false)
        return
      }
      const accelerator = acceleratorFrom(event)
      if (accelerator) {
        setRecording(false)
        onChange(accelerator)
      }
    }
    // Capturing, so the app's own shortcuts don't take the keys first.
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [recording, onChange])

  let label = 'Record shortcut'
  if (recording) {
    label = 'Press a shortcut…'
  } else if (value) {
    label = formatAccelerator(value)
  }

  return (
    <div className="flex items-center gap-1">
      <Button size="sm" variant="secondary" aria-pressed={recording} onClick={() => setRecording(!recording)}>
        {label}
      </Button>
      {value && !recording ? (
        <Button
          size="sm"
          variant="ghost"
          aria-label="Clear shortcut"
          leadingIcon={<X className="h-3.5 w-3.5" />}
          className="w-7 px-0"
          onClick={() => onChange(null)}
        />
      ) : null}
    </div>
  )
}
