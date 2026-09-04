// The chart chrome every panel on this page is made of.
//
// The mark specs live here so they are written once: the 2px of surface between stacked
// fills, the 4px cap on the data-end, and the 24px width limit.

import { useState } from 'react'
import type { ReactNode } from 'react'
import { Rectangle } from 'recharts'

/** Rounded data-end, and the surface gap that separates one stacked fill from the next. */
const CAP = 4
const GAP = 2

/** The widest a bar gets, however few there are. */
export const BAR = 24

/** A row of a bucketed chart: the bucket instant, the numbers, and which stacked key sits
 * on top of it — the only one that gets a rounded cap. */
export type Stacked = { top: string | null }

/** A stacked segment. The gap comes off its own top, so it separates this fill from
 * whatever sits above it; the topmost has nothing above it and keeps its full height. */
export function Segment(props: Record<string, unknown>) {
  const { height, y, payload, dataKey } = props as {
    height: number
    y: number
    payload: Stacked
    dataKey: string
  }
  if (!height) return null
  const isTop = payload.top === dataKey
  // Never shrink a fill so far that it disappears: below ~3px the gap would cost more
  // than it separates, and a bar that is there has to be visible.
  const gap = isTop || height <= GAP + 1 ? 0 : GAP
  return (
    <Rectangle
      {...props}
      y={y + gap}
      height={height - gap}
      radius={isTop ? [CAP, CAP, 0, 0] : 0}
    />
  )
}

export type Key = { label: string; colour: string }

/** Always present for two or more series: identity must never rest on colour alone. A
 * chart with one series has no legend — its title already names what it draws. */
export function Legend({ keys }: { keys: Key[] }) {
  if (keys.length < 2) return null
  return (
    <div className="legend">
      {keys.map((k) => (
        <span key={k.label}>
          <i style={{ background: k.colour }} />
          {k.label}
        </span>
      ))}
    </div>
  )
}

/** The card a chart sits in, with its table twin behind a toggle. The table is not a
 * nicety: three of the light-mode hues sit under 3:1 against the surface, and a readable
 * text version is what makes them legal to use at all. */
export function Panel({
  title,
  sub,
  stale,
  table,
  children,
}: {
  title: string
  sub?: string
  stale: boolean
  table: ReactNode
  children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  return (
    <section className="card">
      <h2>{title}</h2>
      {sub && <p className="sub">{sub}</p>}
      <div className={stale ? 'stale' : undefined}>
        {children}
        <button className="table-toggle" onClick={() => setOpen(!open)}>
          {open ? 'Hide table' : 'Show table'}
        </button>
        {open && <div className="scroll-x">{table}</div>}
      </div>
    </section>
  )
}
