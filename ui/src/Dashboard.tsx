// Which charts are drawn, in what order.
//
// Every card on the page is one entry in one list here, so a chart can be turned off or
// moved without the component that draws it knowing anything about it.
//
// The saved layout is a preference, not a description of the data. It names ids and says
// nothing about what they draw, and that is what lets the two kinds of chart on this page
// coexist: the fixed ones, which are always there, and the structured keys, which exist
// only while a call in the selection carries them.
//
// Two rules follow from that, and they are the whole file:
//
//   An id the layout has never seen is new — a chart added by an upgrade, or a key that
//   arrived with the last sync — and it is drawn. Hiding it would be the dashboard
//   deciding on the reader's behalf that a number they have never seen is not worth
//   seeing.
//
//   An id in the layout with no chart behind it is remembered anyway. It is a key this
//   selection does not carry; narrowing a filter must not quietly delete a preference.

import { useCallback, useEffect, useRef, useState } from 'react'
import * as api from './api'
import type { ChartPref } from './api'
import analysisEntries, { STRUCTURED } from './charts/Analysis'
import EndedGroups from './charts/EndedGroups'
import packEntries from './charts/Pack'
import { dashboardPdf } from './pdf'
import type { Card, Pair } from './pdf'
import PdfButton from './pdf/Button'
import type { Entry } from './charts/entry'
import type { Chart } from './series'

/** A row of the Charts menu: every chart the layout knows about, drawn or not. */
type Row = ChartPref & {
  title: string
  /** Whether this selection has a chart for it. A saved id with nothing behind it stays
   * in the menu so it can still be moved and re-enabled. */
  present: boolean
}

/** What to call a saved id with no chart behind it. Only the structured keys can get
 * here, and their id is the key with a namespace on the front. */
const label = (id: string) => (id.startsWith(STRUCTURED) ? id.slice(STRUCTURED.length) : id)

/** Every chart this page could draw, in the order it ships in. */
function build(chart: Chart, stale: boolean): Entry[] {
  return [
    {
      id: 'ended_groups',
      title: 'Calls by ended group',
      wide: true,
      node: <EndedGroups chart={chart} stale={stale} />,
    },
    ...packEntries(chart.stats, stale),
    ...analysisEntries(chart.stats, stale),
  ]
}

/** The menu's rows: what the layout says, in its order, then everything it has not heard
 * of yet — which is on, because new means drawn. */
function rows(entries: Entry[], layout: ChartPref[]): Row[] {
  const left = new Map(entries.map((e) => [e.id, e]))
  const out: Row[] = layout.map((pref) => {
    const entry = left.get(pref.id)
    left.delete(pref.id)
    return { ...pref, title: entry?.title ?? label(pref.id), present: entry != null }
  })
  for (const entry of entries) {
    if (left.has(entry.id)) out.push({ id: entry.id, on: true, title: entry.title, present: true })
  }
  return out
}

/** The cards to draw: the rows that are on, each one's chart, in the rows' order. */
function drawn(entries: Entry[], list: Row[]): Entry[] {
  const by = new Map(entries.map((e) => [e.id, e]))
  const out: Entry[] = []
  for (const row of list) {
    const entry = by.get(row.id)
    if (row.on && entry) out.push(entry)
  }
  return out
}

export default function Dashboard({
  org,
  chart,
  selection,
  stale,
  onError,
}: {
  org: number
  chart: Chart
  /** Which calls these charts are about, for the header of the downloaded copy. */
  selection: Pair[]
  stale: boolean
  onError: (e: unknown) => void
}) {
  const [layout, setLayout] = useState<ChartPref[]>([])
  const [open, setOpen] = useState(false)
  /** The card each drawn chart is in, by id. A ref and not state: the PDF reads it at the
   * moment of the click and nothing on screen depends on it. Registered by the cards
   * themselves rather than looked up by selector, so the file gets the charts the page
   * actually drew and cannot go stale against the layout. */
  const cards = useRef(new Map<string, HTMLElement>())

  // Per org: two orgs are two dashboards, and one's choices must not survive into the
  // other. What resets the state between them is the remount `App` forces, not this.
  useEffect(() => {
    let live = true
    api
      .dashboard(org)
      .then((l) => live && setLayout(l.charts))
      .catch((e) => live && onError(e))
    return () => {
      live = false
    }
  }, [org, onError])

  const entries = build(chart, stale)
  const list = rows(entries, layout)

  /** The page reorders now and the write follows. A layout is a preference, so waiting on
   * a round trip before honouring a click would cost more than the write is worth — and a
   * write that fails still says so. */
  const save = useCallback(
    (next: Row[]) => {
      const charts = next.map(({ id, on }) => ({ id, on }))
      setLayout(charts)
      api.saveDashboard(org, charts).catch(onError)
    },
    [org, onError],
  )

  const toggle = (i: number) =>
    save(list.map((row, n) => (n === i ? { ...row, on: !row.on } : row)))

  const move = (from: number, to: number) => {
    if (to < 0 || to >= list.length || from === to) return
    const next = [...list]
    next.splice(to, 0, ...next.splice(from, 1))
    save(next)
  }

  const showing = drawn(entries, list)

  /** The cards the PDF is made of: the drawn charts, in their order, each with the DOM node
   * it was drawn into. Read at the click and not before — a card the layout has since
   * turned off has already taken its node out of the map. */
  const captured = () => {
    const out: Card[] = []
    for (const entry of showing) {
      const node = cards.current.get(entry.id)
      if (node) out.push({ node, wide: entry.wide === true })
    }
    return out
  }

  return (
    <>
      <div className="charts-bar">
        <button aria-expanded={open} aria-controls="chart-menu" onClick={() => setOpen(!open)}>
          Charts
        </button>
        {/* The order is the reader's order and the charts are the reader's charts, so the
            file is built from `showing` — the same list, read at the click. */}
        <PdfButton make={() => dashboardPdf(selection, captured())} onError={onError} />
      </div>
      {open && <Menu list={list} onToggle={toggle} onMove={move} />}
      <div className="pack">
        {showing.map((entry) => (
          <div
            key={entry.id}
            className={entry.wide ? 'wide' : undefined}
            ref={(el) => {
              if (el) cards.current.set(entry.id, el)
              else cards.current.delete(entry.id)
            }}
          >
            {entry.node}
          </div>
        ))}
      </div>
    </>
  )
}

function Menu({
  list,
  onToggle,
  onMove,
}: {
  list: Row[]
  onToggle: (i: number) => void
  onMove: (from: number, to: number) => void
}) {
  /** Which row the pointer picked up. A ref and not state: it changes on every drag event
   * and nothing on screen reads it. */
  const held = useRef<number | null>(null)

  return (
    <ul id="chart-menu" className="chart-menu">
      {list.map((row, i) => (
        <li
          key={row.id}
          draggable
          onDragStart={() => (held.current = i)}
          onDragOver={(e) => e.preventDefault()}
          onDrop={() => held.current != null && onMove(held.current, i)}
          onDragEnd={() => (held.current = null)}
        >
          <label>
            <input type="checkbox" checked={row.on} onChange={() => onToggle(i)} />
            {row.title}
          </label>
          {!row.present && <span className="hint">not in this range</span>}
          {/* Dragging is for a pointer and nothing else, so the order is reachable by a
              pair of real buttons too. */}
          <button
            className="nudge"
            disabled={i === 0}
            aria-label={`Move ${row.title} up`}
            onClick={() => onMove(i, i - 1)}
          >
            ↑
          </button>
          <button
            className="nudge"
            disabled={i === list.length - 1}
            aria-label={`Move ${row.title} down`}
            onClick={() => onMove(i, i + 1)}
          >
            ↓
          </button>
        </li>
      ))}
    </ul>
  )
}
