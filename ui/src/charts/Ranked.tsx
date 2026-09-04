// A ranked horizontal bar: one row per name, longest first.
//
// There is no colour encoding here. Length is the whole answer, so every bar wears the
// same hue — colouring by rank would repaint the chart every time a filter changed the
// order, which is the one thing colour must never do.

import type { ReactNode } from 'react'
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Rectangle,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { count } from '../format'
import { BAR, Panel } from './frame'

export type Item = { name: string; value: number; note?: string }

/** How many bars before the tail is folded away. Past this the labels stop being legible
 * and the chart stops being a ranking, so the rest is summed into one honest row. */
const TOP = 10
const REST = 'other'

type Row = { name: string; value: number; note?: string }

/** Longest first, with everything past `TOP` summed into a single named row. Nothing is
 * dropped: the fold is counted, labelled, and says how many names it covers. */
function rank(items: Item[]): Row[] {
  const sorted = [...items].sort((a, b) => b.value - a.value)
  if (sorted.length <= TOP + 1) return sorted
  const tail = sorted.slice(TOP)
  return [
    ...sorted.slice(0, TOP),
    {
      name: REST,
      value: tail.reduce((n, i) => n + i.value, 0),
      note: `${tail.length} more`,
    },
  ]
}

/** The rounded data-end sits on the far end of the bar, away from the baseline. */
function Bristle(props: Record<string, unknown>) {
  return <Rectangle {...props} radius={[0, 4, 4, 0]} />
}

type TipProps = { active?: boolean; payload?: { payload: Row }[]; unit: string }

function Tip({ active, payload, unit }: TipProps) {
  const row = payload?.[0]?.payload
  if (!active || !row) return null
  return (
    <div className="tip">
      <h3>{row.name}</h3>
      <div className="rows">
        <div className="row">
          <i style={{ background: 'var(--s-1)' }} />
          <span className="v">{row.value}</span>
          <span className="k">{unit}</span>
        </div>
      </div>
      {row.note && <p className="reasons">{row.note}</p>}
    </div>
  )
}

export default function Ranked({
  title,
  sub,
  items,
  unit,
  stale,
  empty,
}: {
  title: string
  sub?: string
  items: Item[]
  /** What one unit of the bar is, for the tooltip and the table header. */
  unit: string
  stale: boolean
  /** What to say when there is nothing to rank. Never a chart of zero-length bars. */
  empty: ReactNode
}) {
  const rows = rank(items)

  return (
    <Panel title={title} sub={sub} stale={stale} table={<Table rows={rows} unit={unit} />}>
      {rows.length === 0 ? (
        <p className="notice">{empty}</p>
      ) : (
        <ResponsiveContainer width="100%" height={Math.max(120, rows.length * 32 + 30)}>
          <BarChart
            data={rows}
            layout="vertical"
            margin={{ top: 4, right: 12, bottom: 0, left: 0 }}
          >
            <CartesianGrid horizontal={false} stroke="var(--grid)" />
            <XAxis
              type="number"
              allowDecimals={false}
              tick={{ fill: 'var(--ink-muted)', fontSize: 11 }}
              axisLine={{ stroke: 'var(--axis)' }}
              tickLine={false}
            />
            <YAxis
              type="category"
              dataKey="name"
              width={140}
              tick={{ fill: 'var(--ink-2)', fontSize: 11 }}
              axisLine={false}
              tickLine={false}
            />
            <Tooltip cursor={{ fill: 'var(--wash)' }} content={<Tip unit={unit} />} />
            <Bar
              dataKey="value"
              fill="var(--s-1)"
              maxBarSize={BAR}
              isAnimationActive={false}
              shape={<Bristle />}
            >
              {/* The fold is the one row that is not a single name, so it is greyed —
                  chart chrome, the same grey the ended-group residual wears. */}
              {rows.map((row) => (
                <Cell
                  key={row.name}
                  fill={row.note ? 'var(--g-other)' : 'var(--s-1)'}
                />
              ))}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      )}
    </Panel>
  )
}

function Table({ rows, unit }: { rows: Row[]; unit: string }) {
  return (
    <table>
      <thead>
        <tr>
          <th>name</th>
          <th>{unit}</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.name}>
            <td>
              {row.name}
              {row.note && ` (${row.note})`}
            </td>
            <td>{count(row.value)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
