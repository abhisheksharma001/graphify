// One chart shape, drawn six times: a measure of `/api/stats`'s per-bucket totals over the
// same time axis the ended-group chart uses.
//
// Bars stack; lines overlay. That covers every chart in the pack — a pure stack (cost by
// component), a pure line (tool failures over time), and the latency chart, which is both:
// the components add up to the average turn, and the two percentiles are lines drawn over
// it, because they answer a different question about the same milliseconds.
//
// One axis, always. Two measures of different scale get two charts, never two scales.

import type { ReactNode } from 'react'
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import type { Totals } from '../api'
import { day, full, hour } from '../format'
import { BAR, Legend, Panel, Segment } from './frame'
import type { Key } from './frame'

/** A number the engine reports per bucket. Named by its field, so the chart and the table
 * can never disagree about which number they are showing.
 *
 * The pack's charts name fields of `Totals`. A structured numeric key has no field of its
 * own — the engine cannot know its name at compile time — so it arrives on a series of
 * its own under `avg`, which is why that one name is in here too. */
export type Field = keyof Totals | 'avg'

export type Measure = { field: Field; label: string; colour: string }

/** Anything with a bucket stamp and some of these numbers on it: `Bucket` from the
 * engine, or a structured key's own series. */
export type Series = { bucket: string } & Partial<Record<Field, number | null>>

type Row = { ms: number; top: string | null } & Partial<Record<Field, number | null>>

/** A row per bucket, carrying only the fields this chart draws.
 *
 * A NULL stays NULL. Recharts draws no bar and breaks the line at one, which is exactly
 * right: an hour nothing was measured in is a gap, not a floor. */
function rowsFor(buckets: Series[], fields: Measure[], stack: Measure[]): Row[] {
  return buckets.map((b) => {
    const ms = Date.parse(b.bucket)
    const row: Row = { ms, top: null }
    for (const m of fields) row[m.field] = b[m.field]
    // The stack is drawn in order, so the last member carrying anything is on top.
    for (const m of stack) if ((b[m.field] ?? 0) > 0) row.top = m.field
    return row
  })
}

type TipProps = {
  active?: boolean
  payload?: { payload: Row }[]
  measures: Measure[]
  format: (v: number | null) => string
}

function Tip({ active, payload, measures, format }: TipProps) {
  const row = payload?.[0]?.payload
  if (!active || !row) return null
  // Only what this bucket actually carries. A measure with nothing in it is left out
  // rather than listed as a zero.
  const present = measures.filter((m) => row[m.field] != null)
  if (present.length === 0) return null
  return (
    <div className="tip">
      <h3>{full.format(row.ms)}</h3>
      <div className="rows">
        {present.map((m) => (
          <div key={m.field} className="row">
            <i style={{ background: m.colour }} />
            <span className="v">{format(row[m.field] ?? null)}</span>
            <span className="k">{m.label}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

export default function Bucketed({
  title,
  sub,
  buckets,
  bucketSize,
  stale,
  stack = [],
  lines = [],
  format,
  empty = 'Nothing measured in this range.',
}: {
  title: string
  sub?: string
  buckets: Series[]
  bucketSize: string
  stale: boolean
  /** Drawn as stacked bars, bottom of the stack first. */
  stack?: Measure[]
  /** Drawn as lines over the bars. */
  lines?: Measure[]
  format: (v: number | null) => string
  /** What to say when nothing in the range carried any of these numbers. A chart of
   * zeroes would be a lie; so would a generic message where the reason is specific. */
  empty?: ReactNode
}) {
  const measures = [...stack, ...lines]
  const rows = rowsFor(buckets, measures, stack)
  const hourly = bucketSize === '1h'
  const tick = (ms: number) => (hourly ? hour : day).format(ms)
  const keys: Key[] = measures.map((m) => ({ label: m.label, colour: m.colour }))
  const ring =
    stack.length > 0 ? { strokeWidth: 2, stroke: 'var(--surface)' } : { strokeWidth: 0 }

  // Nothing measured anywhere in the range is not a chart of zeroes.
  const anything = rows.some((r) => measures.some((m) => r[m.field] != null))

  return (
    <Panel
      title={title}
      sub={sub}
      stale={stale}
      table={<Table rows={rows} measures={measures} format={format} />}
    >
      {!anything ? (
        <p className="notice">{empty}</p>
      ) : (
        <>
          <Legend keys={keys} />
          <ResponsiveContainer width="100%" height={220}>
            <ComposedChart data={rows} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
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
                /* Wide enough for a formatted tick with its unit on it: an axis that
                 * wraps "3400 ms" onto two lines is harder to read than a wider gutter. */
                width={64}
                tickFormatter={(v: number) => format(v)}
                tick={{ fill: 'var(--ink-muted)', fontSize: 11 }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip
                cursor={{ fill: 'var(--wash)' }}
                content={<Tip measures={measures} format={format} />}
              />
              {stack.map((m) => (
                <Bar
                  key={m.field}
                  dataKey={m.field}
                  stackId="one"
                  fill={m.colour}
                  maxBarSize={BAR}
                  isAnimationActive={false}
                  shape={<Segment />}
                />
              ))}
              {lines.map((m) => (
                <Line
                  key={m.field}
                  /* Straight, not splined. A curve between two buckets passes through
                   * values nothing measured, which is the one thing this dashboard is
                   * not allowed to draw. */
                  type="linear"
                  dataKey={m.field}
                  stroke={m.colour}
                  strokeWidth={2}
                  isAnimationActive={false}
                  /* A gap in the data is drawn as a gap. Joining across it would invent a
                   * value for an hour nothing was measured in. */
                  connectNulls={false}
                  /* A bucket with empty neighbours is a line segment of no length, so it
                   * has to carry a mark of its own or it is not on the chart at all.
                   *
                   * The ring goes on only where the line crosses bars — a p50 drawn
                   * straight onto a stacked fill of nearly its own value disappears into
                   * it. On a chart that is nothing but the line, the same ring would chop
                   * the stroke into dashes at every bucket. */
                  dot={{ r: 3, ...ring, fill: m.colour }}
                  activeDot={{ r: 5, ...ring }}
                />
              ))}
            </ComposedChart>
          </ResponsiveContainer>
        </>
      )}
    </Panel>
  )
}

function Table({
  rows,
  measures,
  format,
}: {
  rows: Row[]
  measures: Measure[]
  format: (v: number | null) => string
}) {
  return (
    <table>
      <thead>
        <tr>
          <th>bucket</th>
          {measures.map((m) => (
            <th key={m.field}>{m.label}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <tr key={row.ms}>
            <td>{full.format(row.ms)}</td>
            {measures.map((m) => (
              <td key={m.field}>{format(row[m.field] ?? null)}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  )
}
