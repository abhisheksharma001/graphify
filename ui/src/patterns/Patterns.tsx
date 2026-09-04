// The patterns screen: the list on the left, one pattern on the right, and its calls under
// both. Plus the wizard, which is where a new one comes from.
//
// One filter bar scopes all of it, the same one the dashboard uses. That is the whole point
// of the count beside a pattern's name: narrow the window and watch it move. So every
// request below is built from the same query string, and the pattern's chart, the table
// under it and the count in the list are three answers about one set of calls — the engine
// takes all three over the same selection, and the browser counts nothing of its own.
//
// Nothing on this screen spends. The rule half of a pattern is arithmetic over calls that
// are already stored; the only thing in graphify that lets a model read a call is the go
// button in the wizard.

import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import * as api from '../api'
import type { Assistant, Call, Pattern, Stats } from '../api'
import CallTable from '../CallTable'
import PatternChart from '../charts/pattern'
import { SETTLE_MS } from '../filters'
import { count, DASH, full, named } from '../format'
import { patternPdf } from '../pdf'
import type { Pair } from '../pdf'
import PdfButton from '../pdf/Button'
import Editor from './Editor'
import List from './List'
import Wizard from './Wizard'

const percent = (x: number) => `${Math.round(x * 100)}%`

/** What the engine measured for one selected pattern: the buckets its chart is drawn from
 * and the rows its table is. Both from one request, so neither can be the other's
 * leftovers, and both tagged twice over — with the pattern they are about and with the
 * whole query they came from. The two tags answer different questions. A stale query is
 * the same pattern a moment ago, which is worth leaving on screen, dimmed. A stale
 * pattern is somebody else's numbers under this one's name, which is not. */
type Detail = { pattern: number; of: string; stats: Stats; calls: Call[] }

export default function Patterns({
  org,
  assistants,
  query,
  selection,
  bar,
  onError,
}: {
  org: number
  assistants: Assistant[]
  /** The filter bar's query string. The pattern is added to it, never the other way. */
  query: string
  /** Which calls the bar is describing, for the headers of the two files this screen can
   * download. The pattern is added to it here, the same way the query is. */
  selection: Pair[]
  /** The filter bar itself, handed down rather than built here, because the filters belong
   * to the page. This screen only decides where it goes — and that it does not go above
   * the wizard, which picks its own calls and would be ignoring every control on it. */
  bar: ReactNode
  onError: (e: unknown) => void
}) {
  const [selected, setSelected] = useState<number | null>(null)
  const [wizard, setWizard] = useState(false)
  /** Bumped by anything that changed what is stored — a save, a re-apply, or coming back
   * from the wizard. The filters did not move, so this is what makes the loads run again. */
  const [changed, setChanged] = useState(0)
  const [list, setList] = useState<{ of: string; rows: Pattern[] } | null>(null)
  const [detail, setDetail] = useState<Detail | null>(null)

  const listTag = `${query}|${changed}`

  useEffect(() => {
    let live = true
    const timer = setTimeout(() => {
      api
        .patterns(new URLSearchParams(query))
        .then((rows) => live && setList({ of: listTag, rows }))
        .catch((e) => live && onError(e))
    }, SETTLE_MS)
    return () => {
      live = false
      clearTimeout(timer)
    }
  }, [listTag, query, onError])

  const rows = list?.rows ?? []
  /** The pattern on screen: the chosen one while it is still in the list, otherwise the
   * first. Derived rather than corrected after the fact, so there is no moment where a
   * screen with patterns on it is showing none of them. */
  const showing = rows.find((p) => p.id === selected) ?? rows[0] ?? null
  const shown = showing?.id ?? null
  const detailTag = `${listTag}|${shown}`

  useEffect(() => {
    if (shown === null) return
    let live = true
    const params = new URLSearchParams(query)
    params.set('pattern', String(shown))
    // No debounce here: `shown` only moves on a click, and the query reaching this effect
    // has already been through the pause above.
    Promise.all([api.stats(params), api.calls(params)])
      .then(([stats, calls]) => live && setDetail({ pattern: shown, of: detailTag, stats, calls }))
      .catch((e) => live && onError(e))
    return () => {
      live = false
    }
  }, [detailTag, query, shown, onError])

  const listStale = list === null || list.of !== listTag
  /** The measurements, but only while they are this pattern's. Another pattern's numbers
   * under this one's name would be wrong rather than merely old, and dimming them would
   * say they were about to be confirmed. */
  const measured = detail !== null && detail.pattern === shown ? detail : null
  const detailStale = measured === null || measured.of !== detailTag

  if (wizard) {
    return (
      <>
        <button
          type="button"
          className="back"
          onClick={() => {
            setWizard(false)
            setChanged((n) => n + 1)
          }}
        >
          ← Patterns
        </button>
        {/* Keyed by org for the same reason the dashboard is: a half-filled wizard must
            not carry over to somebody else's calls. */}
        <Wizard key={org} org={org} assistants={assistants} onError={onError} />
      </>
    )
  }

  return (
    <>
      {bar}
      <div className="patterns">
        <List
          patterns={rows}
          selected={shown}
          stale={listStale}
          onSelect={setSelected}
          onNew={() => setWizard(true)}
        />

        {showing === null ? (
          <p className="notice">
            {list === null
              ? 'Loading…'
              : 'Nothing saved yet. Start a pattern and this is where it will live.'}
          </p>
        ) : (
          /* Keyed by pattern: a different pattern is a different editor, so nothing of the
             one before is still in the box while this one is being read. */
          <div className="pattern" key={showing.id}>
            <section className="card about">
              <h2>
                {named(showing.id, showing.name)}
                {/* Only once the calls are here. The file's list is the table's rows, so
                    downloading before they land would produce a report that says this
                    pattern matched nothing — one load, and the file cannot describe a set
                    the screen is not showing. */}
                {measured !== null && (
                  <PdfButton
                    make={() => patternPdf(selection, showing, measured.calls)}
                    onError={onError}
                  />
                )}
              </h2>
              {showing.criterion && <p className="sub">{showing.criterion}</p>}
              {/* Each pair in its own box: a `dl` laid out as a grid puts its terms and
                  its values in document order, which reads as a value under the wrong
                  label the moment a row wraps. */}
              <dl>
                <div>
                  <dt>Matched here</dt>
                  <dd>{count(showing.matched)}</dd>
                </div>
                <div>
                  <dt>Agreement</dt>
                  <dd>
                    {showing.agreement == null ? DASH : percent(showing.agreement)}
                    {showing.agreement != null &&
                      showing.sample_size != null &&
                      ` of ${showing.sample_size}`}
                  </dd>
                </div>
                <div>
                  <dt>Learned by</dt>
                  <dd>{showing.model ?? DASH}</dd>
                </div>
                <div>
                  <dt>Learned on</dt>
                  <dd>
                    {showing.created_at ? full.format(new Date(showing.created_at)) : DASH}
                  </dd>
                </div>
              </dl>
            </section>

            {measured === null ? (
              <p className="notice">Loading…</p>
            ) : (
              <PatternChart
                title={showing.chart?.title?.trim() || named(showing.id, showing.name)}
                sub={
                  /* Not "the rule matched": in the two modes with a model in the loop the
                     rule is a prefilter and the model has the last word over the calls it
                     has read, so what is drawn here is the pattern's answer and not one
                     half of it. */
                  showing.mode === null || showing.mode === 'free'
                    ? 'Calls of this selection the rule matched, bucket by bucket.'
                    : 'Calls of this selection this pattern matched, rule and model together.'
                }
                buckets={measured.stats.per_bucket}
                bucketSize={measured.stats.bucket_size}
                kind={showing.chart?.kind}
                stale={detailStale}
              />
            )}

            <Editor pattern={showing} onChanged={() => setChanged((n) => n + 1)} />
          </div>
        )}
      </div>

      {/* The calls themselves, cut to the pattern above. Clicking a different pattern in
          the list is what filters this table — it is the same request with a different
          `pattern=` on it, so the table can never be showing a set the chart is not. */}
      {measured !== null && showing !== null && (
        <CallTable
          rows={measured.calls}
          selection={[...selection, ['Pattern', named(showing.id, showing.name)]]}
          stale={detailStale}
          onError={onError}
        />
      )}
    </>
  )
}
