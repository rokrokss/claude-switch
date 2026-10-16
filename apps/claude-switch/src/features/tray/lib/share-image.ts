import type { Settings, TrayEntry } from '@/lib/types'
import type { TotalSpend } from './total-spend'

import { appSpecs } from '@/lib/app-registry'
import { copyImageToClipboard } from '@/lib/commands'

import { formatCompact, formatCost, formatDeadline, meterHeadline, shownFraction, valuesText } from './format'
import { cardLayout } from './layout'
import { severityOf } from './pace'
import { costPerMillion, shareOf } from './total-spend'

type Tone = 'normal' | 'warning' | 'critical' | null

/**
 * A line of a shared card, in the order it is drawn.
 */
export type ShareLine =
  | { kind: 'meter'; label: string; headline: string; fraction: number; tone: Tone; note: string | null }
  | { kind: 'text'; label: string; value: string }
  | { kind: 'bars'; label: string; values: Array<number> }
  | { kind: 'hero'; value: string }
  | { kind: 'stack'; segments: Array<{ share: number; slot: number | null }> }
  | { kind: 'legend'; name: string; value: string; share: string; slot: number | null }

export type ShareCard = {
  title: string
  /**
   * Beside the title: the plan, or the period.
   */
  badge: string | null
  /**
   * At the title's far side: the app, or what is shown.
   */
  aside: string
  lines: Array<ShareLine>
}

const periodName: Record<Settings['display']['totalSpend']['period'], string> = {
  today: 'Today',
  yesterday: 'Yesterday',
  last30Days: 'Last 30 days',
}

/**
 * A profile's card as shared: its always-visible metrics, as the panel
 * reads them now.
 */
export function profileShareCard(entry: TrayEntry, settings: Settings, now: number): ShareCard {
  const lines: Array<ShareLine> = []
  for (const { metric } of cardLayout(entry, settings.layout).alwaysVisible) {
    if (metric?.kind === 'meter') {
      lines.push({
        kind: 'meter',
        label: metric.label,
        headline: meterHeadline(metric, settings.display.meterStyle),
        fraction: shownFraction(metric, settings.display.meterStyle),
        tone: severityOf(metric.state),
        note: metric.resetsAt
          ? formatDeadline('Resets', new Date(metric.resetsAt), 'countdown', settings.display.timeFormat, new Date(now))
          : null,
      })
    } else if (metric?.kind === 'values') {
      lines.push({ kind: 'text', label: metric.label, value: valuesText(metric) })
    } else if (metric?.kind === 'chart') {
      lines.push({ kind: 'bars', label: metric.label, values: metric.points.map((point) => point.value) })
    }
  }
  return { title: entry.name, badge: entry.plan, aside: appSpecs[entry.app].displayName, lines }
}

/**
 * The total spend card as shared, with what it shows now.
 */
export function totalSpendShareCard(total: TotalSpend, settings: Settings['display']['totalSpend']): ShareCard {
  const value = (cost: number, tokens: number) => {
    if (settings.metric === 'tokens') {
      return `${formatCompact(tokens)} tokens`
    }
    if (settings.metric === 'costPerMillionTokens') {
      const perMillion = costPerMillion(cost, tokens)
      return perMillion === null ? '—' : `${formatCost(perMillion)}/M`
    }
    return formatCost(cost)
  }
  const share = (slice: TotalSpend['slices'][number]) => shareOf(slice, total, settings.metric)
  return {
    title: 'Total spend',
    badge: periodName[settings.period],
    aside: 'Estimated from local logs',
    lines: [
      { kind: 'hero', value: value(total.cost, total.tokens) },
      { kind: 'stack', segments: total.slices.map((slice) => ({ share: share(slice), slot: slice.slot })) },
      ...total.slices.map(
        (slice): ShareLine => ({
          kind: 'legend',
          name: slice.name,
          value: value(slice.cost, slice.tokens),
          share: `${Math.round(share(slice) * 100)}%`,
          slot: slice.slot,
        }),
      ),
    ],
  }
}

/**
 * The outline of claude-switch's pixel-art icon, a cell per character; the
 * same mark the menu bar shows.
 */
const appMark = [
  '..#....#........',
  '.#.#..#.#.......',
  '.#.#..#.#.......',
  '.#.####.#.......',
  '#........#......',
  '#.#....#.#......',
  '#.#....###...#..',
  '#......#.#..#.#.',
  '.#.....#.#..#.#.',
  '..######.####.#.',
  '......#........#',
  '......#.#....#.#',
  '......#.#....#.#',
  '......#........#',
  '.......#......#.',
  '........######..',
]

const width = 360
const margin = 10
const pad = 16
const scale = 2
const lineHeight: Record<ShareLine['kind'], number> = {
  meter: 46,
  text: 22,
  bars: 42,
  hero: 34,
  stack: 16,
  legend: 19,
}
const headerHeight = 34
const footerHeight = 34
const sans = '"Geist Variable", -apple-system, BlinkMacSystemFont, sans-serif'

type Palette = Record<
  'base' | 'card' | 'border' | 'ink' | 'inkSoft' | 'muted' | 'track' | 'green' | 'amber' | 'red',
  string
> & { series: Array<string> }

/**
 * The theme's colours as the page has them now, so the image matches it.
 */
function palette(): Palette {
  const style = getComputedStyle(document.documentElement)
  const token = (name: string) => style.getPropertyValue(`--color-${name}`).trim() || '#808080'
  return {
    base: token('base'),
    card: token('cream'),
    border: token('border-soft'),
    ink: token('ink'),
    inkSoft: token('ink-soft'),
    muted: token('muted-strong'),
    track: token('cream-3'),
    green: token('green'),
    amber: token('amber'),
    red: token('red'),
    series: [1, 2, 3, 4, 5, 6].map((slot) => token(`series-${slot}`)),
  }
}

/**
 * Copies `card` as an image, and says how that went.
 */
export function copyAsImage(card: ShareCard, onNotice: (text: string) => void) {
  copyShareCard(card)
    .then(() => onNotice('Copied as an image.'))
    .catch(() => onNotice("Couldn't copy the card as an image."))
}

/**
 * Draws `card` and puts the image on the clipboard.
 */
async function copyShareCard(card: ShareCard): Promise<void> {
  // Canvas draws in a web font only once it has loaded.
  await document.fonts.ready
  const canvas = drawShareCard(card, new Date())
  const pixels = canvas.getContext('2d')?.getImageData(0, 0, canvas.width, canvas.height)
  if (!pixels) {
    throw new Error('Canvas drawing is unavailable.')
  }
  await copyImageToClipboard(new Uint8Array(pixels.data.buffer), canvas.width, canvas.height)
}

/**
 * `card` drawn as an image, two pixels to the point, with claude-switch's
 * mark and address at its foot.
 */
function drawShareCard(card: ShareCard, now: Date): HTMLCanvasElement {
  const colors = palette()
  const height =
    margin * 2 + headerHeight + card.lines.reduce((sum, line) => sum + lineHeight[line.kind], 0) + footerHeight
  const canvas = document.createElement('canvas')
  canvas.width = width * scale
  canvas.height = height * scale
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    throw new Error('Canvas drawing is unavailable.')
  }
  ctx.scale(scale, scale)
  ctx.fillStyle = colors.base
  ctx.fillRect(0, 0, width, height)
  roundedRect(ctx, margin, margin, width - margin * 2, height - margin * 2, 14)
  ctx.fillStyle = colors.card
  ctx.fill()
  ctx.strokeStyle = colors.border
  ctx.lineWidth = 1
  ctx.stroke()

  const left = margin + pad
  const right = width - margin - pad
  drawHeader(ctx, card, colors, left, right, margin + 24)
  let top = margin + headerHeight
  for (const line of card.lines) {
    drawLine(ctx, line, colors, left, right, top)
    top += lineHeight[line.kind]
  }
  drawFooter(ctx, colors, left, right, top, now)
  return canvas
}

function drawHeader(
  ctx: CanvasRenderingContext2D,
  card: ShareCard,
  colors: Palette,
  left: number,
  right: number,
  y: number,
) {
  ctx.textBaseline = 'alphabetic'
  ctx.font = `600 15px ${sans}`
  ctx.fillStyle = colors.ink
  ctx.textAlign = 'left'
  const title = ellipsized(ctx, card.title, 170)
  ctx.fillText(title, left, y)
  if (card.badge) {
    const badgeLeft = left + ctx.measureText(title).width + 8
    ctx.font = `500 10.5px ${sans}`
    const badgeWidth = ctx.measureText(card.badge).width + 10
    roundedRect(ctx, badgeLeft, y - 12, badgeWidth, 16, 4)
    ctx.fillStyle = colors.track
    ctx.fill()
    ctx.fillStyle = colors.inkSoft
    ctx.fillText(card.badge, badgeLeft + 5, y - 0.5)
  }
  ctx.font = `400 11px ${sans}`
  ctx.fillStyle = colors.muted
  ctx.textAlign = 'right'
  ctx.fillText(card.aside, right, y)
  ctx.textAlign = 'left'
}

function drawLine(
  ctx: CanvasRenderingContext2D,
  line: ShareLine,
  colors: Palette,
  left: number,
  right: number,
  top: number,
) {
  switch (line.kind) {
    case 'meter':
      return drawMeter(ctx, line, colors, left, right, top)
    case 'text':
      labelAndValue(ctx, colors, line.label, line.value, left, right, top + 15)
      return
    case 'bars':
      return drawBars(ctx, line, colors, left, right, top)
    case 'hero':
      ctx.font = `600 16px ${sans}`
      ctx.fillStyle = colors.ink
      ctx.fillText(line.value, left, top + 26)
      return
    case 'stack':
      return drawStack(ctx, line, colors, left, right, top)
    case 'legend':
      return drawLegend(ctx, line, colors, left, right, top)
  }
}

function drawMeter(
  ctx: CanvasRenderingContext2D,
  line: Extract<ShareLine, { kind: 'meter' }>,
  colors: Palette,
  left: number,
  right: number,
  top: number,
) {
  labelAndValue(ctx, colors, line.label, line.headline, left, right, top + 15)
  roundedRect(ctx, left, top + 21, right - left, 5, 2.5)
  ctx.fillStyle = colors.track
  ctx.fill()
  if (line.fraction > 0) {
    roundedRect(ctx, left, top + 21, Math.max((right - left) * line.fraction, 5), 5, 2.5)
    ctx.fillStyle = line.tone
      ? { normal: colors.green, warning: colors.amber, critical: colors.red }[line.tone]
      : colors.muted
    ctx.fill()
  }
  if (line.note) {
    ctx.font = `400 10.5px ${sans}`
    ctx.fillStyle = colors.muted
    ctx.textAlign = 'right'
    ctx.fillText(line.note, right, top + 40)
    ctx.textAlign = 'left'
  }
}

function drawBars(
  ctx: CanvasRenderingContext2D,
  line: Extract<ShareLine, { kind: 'bars' }>,
  colors: Palette,
  left: number,
  right: number,
  top: number,
) {
  labelAndValue(ctx, colors, line.label, '', left, right, top + 15)
  const peak = Math.max(0, ...line.values)
  const step = (right - left) / Math.max(line.values.length, 1)
  line.values.forEach((value, index) => {
    const barHeight = peak > 0 && value > 0 ? Math.max((value / peak) * 18, 1.5) : 0
    ctx.fillStyle = index === line.values.length - 1 ? colors.inkSoft : colors.muted
    ctx.fillRect(left + index * step + 1, top + 38 - barHeight, step - 2, barHeight)
  })
}

function drawStack(
  ctx: CanvasRenderingContext2D,
  line: Extract<ShareLine, { kind: 'stack' }>,
  colors: Palette,
  left: number,
  right: number,
  top: number,
) {
  const gap = 2
  const segments = line.segments.filter((segment) => segment.share > 0)
  const room = right - left - gap * Math.max(segments.length - 1, 0)
  let x = left
  ctx.save()
  roundedRect(ctx, left, top + 3, right - left, 5, 2.5)
  ctx.clip()
  for (const segment of segments) {
    const segmentWidth = Math.max(room * segment.share, 3)
    ctx.fillStyle = seriesColor(colors, segment.slot)
    ctx.fillRect(x, top + 3, segmentWidth, 5)
    x += segmentWidth + gap
  }
  ctx.restore()
}

function drawLegend(
  ctx: CanvasRenderingContext2D,
  line: Extract<ShareLine, { kind: 'legend' }>,
  colors: Palette,
  left: number,
  right: number,
  top: number,
) {
  ctx.beginPath()
  ctx.arc(left + 4, top + 9, 4, 0, Math.PI * 2)
  ctx.fillStyle = seriesColor(colors, line.slot)
  ctx.fill()
  ctx.font = `400 11.5px ${sans}`
  ctx.fillStyle = colors.inkSoft
  ctx.fillText(ellipsized(ctx, line.name, 160), left + 14, top + 13)
  ctx.textAlign = 'right'
  ctx.fillStyle = colors.muted
  ctx.fillText(line.share, right, top + 13)
  ctx.fillStyle = colors.ink
  ctx.fillText(line.value, right - 40, top + 13)
  ctx.textAlign = 'left'
}

function drawFooter(
  ctx: CanvasRenderingContext2D,
  colors: Palette,
  left: number,
  right: number,
  top: number,
  now: Date,
) {
  ctx.fillStyle = colors.border
  ctx.fillRect(left, top + 4, right - left, 1)
  const cell = 1
  ctx.fillStyle = colors.ink
  appMark.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === '#') {
        ctx.fillRect(left + x * cell, top + 12 + y * cell, cell, cell)
      }
    }
  })
  ctx.font = `600 11px ${sans}`
  ctx.fillText('claude-switch', left + 22, top + 24)
  const name = ctx.measureText('claude-switch ').width
  ctx.font = `400 11px ${sans}`
  ctx.fillStyle = colors.muted
  ctx.fillText('· claude-switch.shizue.net', left + 22 + name, top + 24)
  ctx.textAlign = 'right'
  ctx.fillText(new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(now), right, top + 24)
  ctx.textAlign = 'left'
}

function labelAndValue(
  ctx: CanvasRenderingContext2D,
  colors: Palette,
  label: string,
  value: string,
  left: number,
  right: number,
  y: number,
) {
  ctx.font = `400 11.5px ${sans}`
  const valueWidth = ctx.measureText(value).width
  ctx.fillStyle = colors.inkSoft
  ctx.textAlign = 'right'
  ctx.fillText(value, right, y)
  ctx.textAlign = 'left'
  ctx.font = `500 12.5px ${sans}`
  ctx.fillStyle = colors.ink
  ctx.fillText(ellipsized(ctx, label, right - left - valueWidth - 12), left, y)
}

function seriesColor(colors: Palette, slot: number | null): string {
  return slot === null ? colors.muted : (colors.series[slot - 1] ?? colors.muted)
}

/**
 * `text`, cut with an ellipsis to fit `maxWidth` in the current font.
 */
function ellipsized(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (ctx.measureText(text).width <= maxWidth) {
    return text
  }
  let cut = text
  while (cut.length > 1 && ctx.measureText(`${cut}…`).width > maxWidth) {
    cut = cut.slice(0, -1)
  }
  return `${cut}…`
}

function roundedRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath()
  ctx.roundRect(x, y, w, h, r)
}
