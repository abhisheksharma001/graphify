// The dashboard: one filter row, one chart, and the two states that come before them.
//
// The filters own the slice. Every request below them is built from the same `Filters`, so
// the chart, its table, and its subtitle can never describe different sets of calls.

import { useCallback, useEffect, useState } from 'react'
import * as api from './api'
import { Unauthorized } from './api'
import type { Assistant, Org } from './api'
import FilterBar from './FilterBar'
import Login from './Login'
import EndedGroups from './charts/EndedGroups'
import Pack from './charts/Pack'
import { initial, toParams } from './filters'
import type { Filters } from './filters'
import { load } from './series'
import type { Chart } from './series'

/** Typing in the call-ID or last box should not fire a request per keystroke. */
const SETTLE_MS = 250

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

export default function App() {
  const [signedOut, setSignedOut] = useState(false)
  const [orgs, setOrgs] = useState<Org[] | null>(null)
  const [assistants, setAssistants] = useState<Assistant[]>([])
  const [filters, setFilters] = useState<Filters>(initial)
  /** The chart, tagged with the query that produced it. Comparing that tag to the current
   * query is what "stale" means, so there is no loading flag to keep in step with it. */
  const [chart, setChart] = useState<{ query: string; data: Chart } | null>(null)
  const [error, setError] = useState<string | null>(null)

  const fail = useCallback((e: unknown) => {
    if (e instanceof Unauthorized) setSignedOut(true)
    else setError(message(e))
  }, [])

  // Orgs first: until there is one, there is nothing to filter by.
  const loadOrgs = useCallback(() => {
    api
      .orgs()
      .then((list) => {
        setSignedOut(false)
        setOrgs(list)
        setFilters((f) => (f.org == null && list.length > 0 ? { ...f, org: list[0].id } : f))
      })
      .catch(fail)
  }, [fail])

  useEffect(loadOrgs, [loadOrgs])

  useEffect(() => {
    if (filters.org == null) return
    let live = true
    api
      .assistants(filters.org)
      .then((list) => live && setAssistants(list))
      .catch(fail)
    return () => {
      live = false
    }
  }, [filters.org, fail])

  // The whole filter set as one string, so an edit that changes nothing on the wire —
  // retyping the same call id, say — does not cost a round trip.
  const query = filters.org == null ? null : toParams(filters).toString()

  useEffect(() => {
    if (query == null) return
    let live = true
    const timer = setTimeout(() => {
      load(new URLSearchParams(query))
        .then((data) => {
          if (!live) return
          setChart({ query, data })
          setError(null)
        })
        .catch((e) => live && fail(e))
    }, SETTLE_MS)
    return () => {
      live = false
      clearTimeout(timer)
    }
  }, [query, fail])

  const stale = chart !== null && chart.query !== query

  if (signedOut) return <Login onDone={loadOrgs} />

  return (
    <div className="page">
      <header className="top">
        <h1 className="wordmark">graphify</h1>
        <span className="spacer" />
        {stale && <span className="hint">loading…</span>}
      </header>

      {error && <p className="error">{error}</p>}

      {orgs === null ? (
        <p className="notice">Loading…</p>
      ) : orgs.length === 0 ? (
        <p className="notice">
          No orgs yet, so there is nothing to chart. Creating one from the dashboard comes
          with the settings screen; until then, POST <code>{'{"name": "acme"}'}</code> to{' '}
          <code>/api/orgs</code>, then sync.
        </p>
      ) : (
        <>
          <FilterBar
            filters={filters}
            orgs={orgs}
            assistants={assistants}
            onChange={setFilters}
          />
          {/* The previous render stays on screen while the next one loads: no skeleton,
              no layout jump, and nothing on screen that is not a real number. */}
          {chart ? (
            <>
              <EndedGroups chart={chart.data} stale={stale} />
              <Pack stats={chart.data.stats} stale={stale} />
            </>
          ) : (
            <p className="notice">Loading…</p>
          )}
        </>
      )}
    </div>
  )
}
