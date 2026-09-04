// Calls per bucket, stacked by ended group.
//
// One colour per group, fixed; the raw `endedReason` behind a group appears when you hover
// its segment. Below the chart is the same numbers as a table, which is what makes the
// three light-mode hues that sit under 3:1 against the surface legal to use at all.

import { useState } from 'react'
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { day, full, hour, money } from '../format'
import { colour } from '../groups'
import type { Group } from '../groups'
import type { Chart, Series } from '../series'
import { BAR, Segment } from './frame'

type Row = {
  ms: number
  cost: number | null
  total: number
  /** The topmost segment with anything in it — the only one that gets a rounded cap. */
  top: Group | null
} & Partial<Record<Group, number>>

function rowsFor(chart: Chart): Row[] {
  return chart.buckets.map(({ ms, cost }) => {
    const row: Row = { ms, cost, total: 0, top: null }
    for (const s of chart.series) {
      const n = s.counts.get(ms) ?? 0
      row[s.group] = n
      row.total += n
      // The series are in stack order, so the last one to carry anything is the top.
      if (n > 0) row.top = s.group
    }
    return row
  })
}

type TipProps = {
  active?: boolean
  payload?: { payload: Row }[]
  chart: Chart
  hovered: Group | null
}

function Tip({ active, payload, chart, hovered }: TipProps) {
  const row = payload?.[0]?.payload
  if (!active || !row) return null
  const present = chart.series.filter((s) => (row[s.group] ?? 0) > 0)
  const reasons = hovered
    ? chart.series.find((s) => s.group === hovered)?.reasons.get(row.ms)
    : undefined
  return (
    <div className="tip">
      <h3>{full.format(row.ms)}</h3>
      <div className="rows">
        {present.map((s) => (
          <div key={s.group} className={`row${s.group === hovered ? ' on' : ''}`}>
            <i style={{ background: colour(s.group) }} />
            <span className="v">{row[s.group]}</span>
            <span className="k">{s.group}</span>
          </div>
        ))}
        <div className="row">
          <i />
          <span className="v">{money(row.cost)}</span>
          <span className="k">cost</span>
        </div>
      </div>
      {reasons && reasons.size > 0 && (
        <p className="reasons">
          <b>{hovered}</b> —{' '}
          {[...reasons]
            .sort((a, b) => b[1] - a[1])
            .map(([reason, n]) => `${reason} (${n})`)
            .join(', ')}
        </p>
      )}
    </div>
  )
}

function Table({ chart, rows }: { chart: Chart; rows: Row[] }) {
  return (
    <div className="scroll-x">
      <table>
        <thead>
          <tr>
            <th>bucket</th>
            {chart.series.map((s) => (
              <th key={s.group}>{s.group}</th>
            ))}
            <th>calls</th>
            <th>cost</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.ms}>
              <td>{full.format(row.ms)}</td>
              {chart.series.map((s) => (
                <td key={s.group}>{row[s.group]}</td>
              ))}
              <td>{row.total}</td>
              <td>{money(row.cost)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export default function EndedGroups({ chart, stale }: { chart: Chart; stale: boolean }) {
  const [hovered, setHovered] = useState<Group | null>(null)
  const [table, setTable] = useState(false)
  const rows = rowsFor(chart)
  const hourly = chart.bucketSize === '1h'
  const tick = (ms: number) => (hourly ? hour : day).format(ms)

  return (
    <section className="card">
      <h2>Calls by ended group</h2>
      <p className="sub">
        {chart.rows.length} call{chart.rows.length === 1 ? '' : 's'} ·{' '}
        {hourly ? 'hourly' : 'daily'} buckets
        {chart.undated > 0 && ` · ${chart.undated} with no start time, not on the axis`}
        {chart.capped && ' · capped by "last", raise it to see more'}
      </p>

      {chart.series.length === 0 ? (
        <p className="notice">No calls in this range.</p>
      ) : (
        <div className={stale ? 'stale' : undefined}>
          <Legend series={chart.series} />
          <ResponsiveContainer width="100%" height={300}>
            <BarChart data={rows} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
              <CartesianGrid vertical={false} stroke="var(--grid)" />
              <XAxis
                dataKey="ms"
                tickFormatter={tick}
                interval="preserveStartEnd"
                tick={{ fill: 'var(--ink-muted)', fontSize: 11 }}
                axisLine={{ stroke: 'var(--axis)' }}
                tickLine={false}
              />
              <YAxis
                allowDecimals={false}
                width={40}
                tick={{ fill: 'var(--ink-muted)', fontSize: 11 }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip
                cursor={{ fill: 'var(--wash)' }}
                content={<Tip chart={chart} hovered={hovered} />}
              />
              {chart.series.map((s) => (
                <Bar
                  key={s.group}
                  dataKey={s.group}
                  stackId="ended"
                  fill={colour(s.group)}
                  maxBarSize={BAR}
                  isAnimationActive={false}
                  shape={<Segment />}
                  onMouseEnter={() => setHovered(s.group)}
                  onMouseLeave={() => setHovered(null)}
                />
              ))}
            </BarChart>
          </ResponsiveContainer>
          <button className="table-toggle" onClick={() => setTable(!table)}>
            {table ? 'Hide table' : 'Show table'}
          </button>
          {table && <Table chart={chart} rows={rows} />}
        </div>
      )}
    </section>
  )
}

/** Always present: identity must never rest on colour alone. */
function Legend({ series }: { series: Series[] }) {
  return (
    <div className="legend">
      {series.map((s) => (
        <span key={s.group}>
          <i style={{ background: colour(s.group) }} />
          {s.group === 'other' ? 'other (transport, start-error, other)' : s.group}
          {' · '}
          {s.total}
        </span>
      ))}
    </div>
  )
}
