// The calls themselves.
//
// Every chart on this page is a summary of these rows, so they are the same rows: the
// table reads the selection `series.ts` already loaded rather than asking for it again.
// Two requests for one selection is two chances for them to disagree.
//
// A row is a way in, not a destination. Clicking one opens the drawer, which is where
// everything the table had to leave out lives.

import { useState } from 'react'
import type { Call } from './api'
import CallDrawer from './CallDrawer'
import { DASH, full, money, seconds } from './format'
import { colour, display } from './groups'

/** A `boolean | null` as text. `false` is an answer and says so; only NULL is a dash. */
const yesNo = (v: boolean | null) => (v === null ? DASH : v ? 'yes' : 'no')

/** Tool calls and how many of them failed, as one fact. A call that made no tool calls
 * made no failed ones either, so the failure count is only worth its own words when
 * there is one. */
function tools(row: Call): string {
  if (row.tool_calls === null) return DASH
  const failed = row.tool_failures ?? 0
  return failed > 0 ? `${row.tool_calls} · ${failed} failed` : String(row.tool_calls)
}

export default function CallTable({
  rows,
  stale,
  onError,
}: {
  rows: Call[]
  stale: boolean
  onError: (e: unknown) => void
}) {
  /** The call the drawer is showing, by id. Null is closed. */
  const [open, setOpen] = useState<string | null>(null)

  return (
    <section className="card calls">
      <h2>Calls</h2>
      <p className="sub">
        {rows.length} call{rows.length === 1 ? '' : 's'} in this selection, newest first.
        Open one for its transcript and its tool calls.
      </p>
      <div className={stale ? 'stale' : undefined}>
        <div className="scroll-x">
          <table>
            <thead>
              <tr>
                <th>Started</th>
                <th className="name">Assistant</th>
                <th>Duration</th>
                <th className="ended">Ended</th>
                <th>Tools</th>
                <th>Transferred</th>
                <th>Cost</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id} onClick={() => setOpen(row.id)}>
                  {/* The button, not the row, is what a keyboard reaches: a pointer can
                      hit anywhere along the row, and the two open the same drawer. */}
                  <td>
                    <button className="row-open" onClick={() => setOpen(row.id)}>
                      {row.created_at ? full.format(new Date(row.created_at)) : DASH}
                    </button>
                  </td>
                  <td className="name">{row.assistant_name ?? row.assistant_id ?? DASH}</td>
                  <td>{seconds(row.duration_s)}</td>
                  {/* The swatch carries the group; the words carry the reason. Text never
                      wears the data colour — three of the light-mode hues would not pass
                      contrast as ink, and a reason has to be readable. */}
                  <td className="ended">
                    {row.ended_reason || row.ended_group ? (
                      <>
                        <i
                          style={{
                            background: colour(display(row.ended_group ?? 'unknown')),
                          }}
                        />
                        {row.ended_reason ?? row.ended_group}
                      </>
                    ) : (
                      DASH
                    )}
                  </td>
                  <td>{tools(row)}</td>
                  <td>{yesNo(row.transferred)}</td>
                  <td>{money(row.cost)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {rows.length === 0 && <p className="hint">No calls in this selection.</p>}
      </div>
      {open && (
        /* Keyed: opening a second call is a second drawer, not the first one with its
           contents swapped, so nothing of the previous call can still be on screen while
           this one is loading. */
        <CallDrawer key={open} id={open} onClose={() => setOpen(null)} onError={onError} />
      )}
    </section>
  )
}
