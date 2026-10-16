import type { DragEndEvent, DragOverEvent } from '@dnd-kit/core'
import type { SegmentedOption } from '@/design'
import type { Settings, TrayEntry } from '@/lib/types'
import type { CustomizeRow, Place } from '../lib/customize'

import { useEffect, useState } from 'react'

import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors } from '@dnd-kit/core'
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import { GripVertical, Star, Undo2 } from 'lucide-react'

import { Button, cn, Segmented } from '@/design'
import { BrandMark } from '@/features/profiles/components/brand-mark'
import { canStar, starsFor, toggleStar } from '@/features/tray/lib/stars'
import { useAppState } from '@/lib/app-state/use-app-state'
import { getTrayUsage, hapticTick } from '@/lib/commands'

import { customizeRows, layoutOf, moveTo, reorder, withoutLayout } from '../lib/customize'
import { SettingsSection } from './settings-card'

type Customization = Pick<Settings, 'layout' | 'menuBar'>

/**
 * Undo reaches this many changes back, for this visit to Settings.
 */
const undoDepth = 40

const placeOptions: ReadonlyArray<SegmentedOption<Place>> = [
  { value: 'alwaysVisible', label: 'Always' },
  { value: 'onDemand', label: 'On demand' },
  { value: 'off', label: 'Off' },
]

const groups: ReadonlyArray<{ place: Place; title: string }> = [
  { place: 'alwaysVisible', title: 'Always visible' },
  { place: 'onDemand', title: 'On demand' },
  { place: 'off', title: 'Off' },
]

/**
 * What each profile's card in the menu bar panel shows, in what order, and
 * which two of its metrics the menu bar itself shows.
 */
export function CustomizeSection() {
  const appState = useAppState()
  const { layout, menuBar } = appState.state.settings
  const entries = useEntries()
  const [history, setHistory] = useState<Array<Customization>>([])

  function save(next: Customization) {
    setHistory((past) => [...past.slice(-(undoDepth - 1)), { layout, menuBar }])
    void appState.update({ settings: next })
  }

  function undo() {
    const previous = history[history.length - 1]
    if (!previous) {
      return
    }
    setHistory((past) => past.slice(0, -1))
    void appState.update({ settings: previous })
  }

  useUndoKey(history.length > 0, undo)

  return (
    <SettingsSection
      title="Panel and menu bar"
      action={
        <Button
          size="sm"
          variant="ghost"
          disabled={history.length === 0}
          leadingIcon={<Undo2 className="h-3.5 w-3.5" />}
          aria-keyshortcuts="Meta+Z"
          onClick={undo}
        >
          Undo
        </Button>
      }
    >
      {entries === null ? <Note>Loading the profiles…</Note> : null}
      {entries?.length === 0 ? <Note>No profile reports usage yet.</Note> : null}
      {entries?.map((entry) => (
        <ProfileCustomizer key={entry.id} entry={entry} layout={layout} menuBar={menuBar} onSave={save} />
      ))}
    </SettingsSection>
  )
}

function Note({ children }: { children: string }) {
  return <p className="px-4 py-3 text-[12px] text-muted">{children}</p>
}

type CustomizerProps = Customization & {
  entry: TrayEntry
  onSave: (next: Customization) => void
}

function ProfileCustomizer({ entry, layout, menuBar, onSave }: CustomizerProps) {
  const rows = customizeRows(entry, layout)
  const stars = starsFor(menuBar, entry.id)
  const [notice, setNotice] = useState<string | null>(null)

  function commit(next: Array<CustomizeRow>) {
    onSave({ layout: { ...layout, profiles: { ...layout.profiles, [entry.id]: layoutOf(next) } }, menuBar })
  }

  function star(id: string) {
    const result = toggleStar(menuBar, entry.id, id)
    if ('refused' in result) {
      setNotice(result.refused)
      return
    }
    setNotice(null)
    onSave({ layout, menuBar: result.menuBar })
  }

  function reset() {
    const { [entry.id]: _dropped, ...otherStars } = menuBar.stars
    onSave({ layout: withoutLayout(layout, entry.id), menuBar: { ...menuBar, stars: otherStars } })
  }

  return (
    <div className="border-b border-border-soft px-4 py-3 last:border-b-0">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <BrandMark app={entry.app} size={14} />
          <span className="truncate text-[13px] font-semibold text-ink">{entry.name}</span>
        </div>
        <Button size="sm" variant="ghost" onClick={reset}>
          Reset
        </Button>
      </div>
      {notice ? (
        <p role="status" className="mb-2 text-[11.5px] text-amber">
          {notice}
        </p>
      ) : null}
      {groups.map((group) => (
        <PlaceGroup
          key={group.place}
          title={group.title}
          rows={rows.filter((row) => row.place === group.place)}
          stars={stars}
          onReorder={(activeId, overId) => commit(reorder(rows, activeId, overId))}
          onPlace={(id, place) => commit(moveTo(rows, id, place))}
          onStar={star}
        />
      ))}
    </div>
  )
}

type GroupProps = {
  title: string
  rows: Array<CustomizeRow>
  stars: Array<string>
  onReorder: (activeId: string, overId: string) => void
  onPlace: (id: string, place: Place) => void
  onStar: (id: string) => void
}

function PlaceGroup({ title, rows, stars, onReorder, onPlace, onStar }: GroupProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  if (rows.length === 0) {
    return null
  }
  function handleDragEnd({ active, over }: DragEndEvent) {
    if (over && active.id !== over.id) {
      onReorder(String(active.id), String(over.id))
    }
  }
  return (
    <div className="mt-2">
      <div className="mb-1 font-mono text-[10px] uppercase tracking-[0.08em] text-muted-strong">{title}</div>
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragOver={snapTick} onDragEnd={handleDragEnd}>
        <SortableContext items={rows.map((row) => row.id)} strategy={verticalListSortingStrategy}>
          <div className="rounded-lg border border-border-soft">
            {rows.map((row) => (
              <SortableMetricRow
                key={row.id}
                row={row}
                starred={stars.includes(row.id)}
                onPlace={(place) => onPlace(row.id, place)}
                onStar={() => onStar(row.id)}
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>
    </div>
  )
}

type RowProps = {
  row: CustomizeRow
  starred: boolean
  onPlace: (place: Place) => void
  onStar: () => void
}

function SortableMetricRow({ row, starred, onPlace, onStar }: RowProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: row.id })
  return (
    <div
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        'flex items-center gap-2 border-b border-border-soft bg-white px-2 py-1.5 last:border-b-0 dark:bg-cream-2',
        isDragging && 'relative z-10 shadow-md',
      )}
    >
      <button
        type="button"
        aria-label={`Move ${row.label}`}
        className="cursor-grab rounded p-0.5 text-muted-strong hover:text-ink"
        {...attributes}
        {...listeners}
      >
        <GripVertical aria-hidden className="h-3.5 w-3.5" />
      </button>
      <span className="min-w-0 flex-1 truncate text-[12.5px] text-ink">
        {row.label}
        {row.reported ? null : <span className="ml-2 text-[11px] text-muted">no data now</span>}
      </span>
      <button
        type="button"
        aria-label={starred ? `Remove ${row.label} from the menu bar` : `Star ${row.label} for the menu bar`}
        aria-pressed={starred}
        disabled={!starred && !canStar(row.id)}
        className="cursor-pointer rounded p-1 text-muted-strong hover:text-ink disabled:cursor-default disabled:opacity-30"
        onClick={onStar}
      >
        <Star aria-hidden className={cn('h-3.5 w-3.5', starred && 'fill-current text-amber')} />
      </button>
      <Segmented<Place>
        ariaLabel={`Where ${row.label} shows`}
        size="small"
        options={placeOptions}
        value={row.place}
        onChange={onPlace}
      />
    </div>
  )
}

/**
 * When the last snap tap was played, so a fast drag doesn't buzz.
 */
let lastTick = 0

/**
 * A trackpad tap each time the dragged row moves to another slot.
 */
function snapTick({ over }: DragOverEvent) {
  const now = Date.now()
  if (over && now - lastTick >= 120) {
    lastTick = now
    hapticTick().catch(() => {
      // No Force Touch trackpad, or not on macOS: the tap is a nicety.
    })
  }
}

/**
 * The profiles the menu bar reports on, as of its last fetch.
 */
function useEntries(): Array<TrayEntry> | null {
  const [entries, setEntries] = useState<Array<TrayEntry> | null>(null)
  useEffect(() => {
    void getTrayUsage().then((loaded) => setEntries(loaded ?? []))
  }, [])
  return entries
}

/**
 * ⌘Z undoes the last change while there is one to undo.
 */
function useUndoKey(enabled: boolean, onUndo: () => void) {
  useEffect(() => {
    if (!enabled) {
      return
    }
    function onKey(event: KeyboardEvent) {
      const target = event.target instanceof Element ? event.target : null
      if (event.metaKey && !event.shiftKey && event.key === 'z' && !target?.closest('input, textarea')) {
        event.preventDefault()
        onUndo()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [enabled, onUndo])
}
