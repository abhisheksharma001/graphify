// The patterns this org has, and how many calls of the current selection each one matched.
//
// The list is never filtered — every pattern is in it, including the ones this window holds
// nothing for, which read 0. A pattern that vanished when you narrowed the range would look
// deleted, and the whole point of the count is to be able to see it fall.
//
// The count is a fact about the filter bar above, not about the pattern: the same rule
// reads 4 over an hour and 260 over a week. So the heading says which.

import type { Pattern } from '../api'
import { count, named } from '../format'

export default function List({
  patterns,
  selected,
  stale,
  onSelect,
  onNew,
}: {
  patterns: Pattern[]
  selected: number | null
  stale: boolean
  onSelect: (id: number) => void
  onNew: () => void
}) {
  return (
    <nav className="pattern-list card">
      <h2>Patterns</h2>
      <p className="sub">Matched calls in this selection.</p>
      <button type="button" className="new-pattern" onClick={onNew}>
        New pattern
      </button>
      {patterns.length === 0 ? (
        <p className="hint">
          No patterns yet. A pattern is one line of plain English turned into a rule the
          engine re-counts every day for nothing.
        </p>
      ) : (
        <ul className={stale ? 'stale' : undefined}>
          {patterns.map((p) => (
            <li key={p.id}>
              {/* `aria-current`, not a class alone: which pattern is showing is a fact
                  about the page, and a reader who cannot see the fill still gets told. */}
              <button
                type="button"
                aria-current={p.id === selected ? 'true' : undefined}
                onClick={() => onSelect(p.id)}
              >
                <span className="what">{named(p.id, p.name)}</span>
                <span className="n">{count(p.matched)}</span>
                {/* The mode is only worth saying when it is one that can spend. */}
                {p.mode !== null && p.mode !== 'free' && (
                  <span className="mode">{p.mode}</span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </nav>
  )
}
