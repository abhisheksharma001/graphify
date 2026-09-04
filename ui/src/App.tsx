// The page: one filter row, the charts, and the two states that come before them.
//
// The filters own the slice. Every request below them is built from the same `Filters`, so
// the chart, its table, and its subtitle can never describe different sets of calls. Which
// of the charts are drawn, and in what order, is `Dashboard`'s — a preference, saved per
// org, and no business of the filters.
//
// The charts summarise the selection; the table under them is the selection. Both are drawn
// from one load, so neither can be describing calls the other is not.

import { useCallback, useEffect, useState } from 'react'
import * as api from './api'
import { Unauthorized } from './api'
import type { Assistant, Org } from './api'
import Ask from './Ask'
import CallTable from './CallTable'
import Dashboard from './Dashboard'
import FilterBar from './FilterBar'
import Login from './Login'
import Settings from './Settings'
import Patterns from './patterns/Patterns'
import { initial, SETTLE_MS, toParams } from './filters'
import type { Filters } from './filters'
import { load } from './series'
import type { Chart } from './series'

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** Which of the three screens is showing. Not a route: graphify is one page served from
 * one binary, and a URL to a settings screen is not a thing anyone needs to share. */
type View = 'dashboard' | 'patterns' | 'ask' | 'settings'

const TABS: Record<View, string> = {
  dashboard: 'Dashboard',
  patterns: 'Patterns',
  ask: 'Ask',
  settings: 'Settings',
}

export default function App() {
  const [view, setView] = useState<View>('dashboard')
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
        <nav>
          {(Object.keys(TABS) as View[]).map((v) => (
            // `aria-current`, not a class alone: which screen you are on is a fact about
            // the page, and a reader who cannot see the underline still gets told.
            <button
              key={v}
              type="button"
              className="tab"
              aria-current={view === v ? 'page' : undefined}
              onClick={() => setView(v)}
            >
              {TABS[v]}
            </button>
          ))}
        </nav>
        <span className="spacer" />
        {stale && view === 'dashboard' && <span className="hint">loading…</span>}
      </header>

      {error && <p className="error">{error}</p>}

      {orgs === null ? (
        <p className="notice">Loading…</p>
      ) : view === 'settings' ? (
        <Settings orgs={orgs} onOrgs={loadOrgs} onError={fail} />
      ) : orgs.length === 0 ? (
        <p className="notice">
          No orgs yet, so there is nothing to chart. Add one on the settings screen: a
          name and a Vapi key, then sync.
        </p>
      ) : view === 'ask' ? (
        query == null ? (
          <p className="notice">Loading…</p>
        ) : (
          /* The same bar again, and for the same reason the patterns screen gets one: a
             question is about a selection, and the only way to say which one is the
             controls that made it. */
          <Ask
            key={filters.org}
            query={query}
            bar={
              <FilterBar
                filters={filters}
                orgs={orgs}
                assistants={assistants}
                onChange={setFilters}
              />
            }
            onError={fail}
          />
        )
      ) : view === 'patterns' ? (
        filters.org == null || query == null ? (
          <p className="notice">Loading…</p>
        ) : (
          /* The same bar as the dashboard's, because a pattern's count is a count of the
             calls on screen and the two have to be scoping one selection. Handed to the
             screen rather than drawn above it: the wizard picks its own calls, and a bar
             it ignores has no business over the top of it. */
          <Patterns
            key={filters.org}
            org={filters.org}
            assistants={assistants}
            query={query}
            bar={
              <FilterBar
                filters={filters}
                orgs={orgs}
                assistants={assistants}
                onChange={setFilters}
              />
            }
            onError={fail}
          />
        )
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
          {chart && filters.org != null ? (
            /* Keyed by org: a different org is a different dashboard, and remounting is
               what guarantees none of the previous one's layout is still on screen while
               this one's is being fetched. */
            <>
              <Dashboard
                key={filters.org}
                org={filters.org}
                chart={chart.data}
                stale={stale}
                onError={fail}
              />
              <CallTable rows={chart.data.rows} stale={stale} onError={fail} />
            </>
          ) : (
            <p className="notice">Loading…</p>
          )}
        </>
      )}
    </div>
  )
}
