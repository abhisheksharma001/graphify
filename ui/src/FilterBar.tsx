// One row of controls above everything they scope.
//
// Ordinary form controls, styled to match the chart chrome — a filter is not a chart mark.
// The range comes first because it is the one every reader reaches for.

import type { Assistant, Org } from './api'
import { LASTS, WINDOWS } from './filters'
import type { Filters } from './filters'

type Props = {
  filters: Filters
  orgs: Org[]
  assistants: Assistant[]
  onChange: (next: Filters) => void
}

export default function FilterBar({ filters, orgs, assistants, onChange }: Props) {
  const set = (patch: Partial<Filters>) => onChange({ ...filters, ...patch })

  return (
    <div className="filters">
      <div className="field">
        <label htmlFor="org">Org</label>
        <select
          id="org"
          value={filters.org ?? ''}
          onChange={(e) => set({ org: Number(e.target.value), assistantIds: [] })}
        >
          {orgs.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
      </div>

      <div className="field">
        <span className="label">Window</span>
        <div className="presets">
          {WINDOWS.map((w) => (
            <button
              key={w}
              type="button"
              aria-pressed={filters.window === w}
              // A preset and a custom range are two answers to the same question, so
              // choosing one clears the other rather than quietly layering on top of it.
              onClick={() => set({ window: w, since: '', until: '' })}
            >
              {w}
            </button>
          ))}
        </div>
      </div>

      <div className="field">
        <label htmlFor="since">Since</label>
        <input
          id="since"
          className="since-until"
          type="datetime-local"
          value={filters.since}
          onChange={(e) => set({ since: e.target.value, window: null })}
        />
      </div>

      <div className="field">
        <label htmlFor="until">Until</label>
        <input
          id="until"
          className="since-until"
          type="datetime-local"
          value={filters.until}
          onChange={(e) => set({ until: e.target.value, window: null })}
        />
      </div>

      <div className="field">
        <label htmlFor="last">Last</label>
        <div className="presets">
          {LASTS.map((n) => (
            <button
              key={n}
              type="button"
              aria-pressed={filters.last === n}
              onClick={() => set({ last: n })}
            >
              {n}
            </button>
          ))}
          <input
            id="last"
            type="number"
            min="1"
            className="last-custom"
            value={filters.last}
            onChange={(e) => set({ last: e.target.value })}
          />
        </div>
      </div>

      <div className="field">
        <label htmlFor="call">Call ID</label>
        <input
          id="call"
          type="text"
          placeholder="one call"
          value={filters.callId}
          onChange={(e) => set({ callId: e.target.value })}
        />
      </div>

      <div className="field assistants">
        <label htmlFor="assistants">Assistants</label>
        <select
          id="assistants"
          multiple
          value={filters.assistantIds}
          onChange={(e) =>
            set({ assistantIds: [...e.target.selectedOptions].map((o) => o.value) })
          }
        >
          {assistants.map((a) => (
            <option key={a.id} value={a.id}>
              {a.name ?? a.id}
            </option>
          ))}
        </select>
        <span className="hint">
          {filters.assistantIds.length === 0
            ? 'all assistants'
            : `${filters.assistantIds.length} selected`}
        </span>
      </div>
    </div>
  )
}
