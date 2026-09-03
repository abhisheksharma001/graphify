// The filter bar's state, and the query string it becomes.
//
// One source of truth for every chart below it, so the numbers on screen always describe
// the same slice.

export type Filters = {
  org: number | null
  assistantIds: string[]
  /** A preset span like `1d`. Null while a custom range is in force. */
  window: string | null
  /** `datetime-local` values — local wall time, converted to UTC on the way out. */
  since: string
  until: string
  /** How many of the newest calls to select. Never empty on the wire — see `toParams`. */
  last: string
  callId: string
}

export const WINDOWS = ['5h', '7h', '1d'] as const

export const LASTS = ['250', '500'] as const

/** What an empty box means. `/api/calls` is a page, and a page with no size is a page of
 * 200 — a cap the reader never asked for and would not see. Naming one here means the
 * chart can always say whether the cap is what ended the selection. */
export const DEFAULT_LAST = '250'

export const initial: Filters = {
  org: null,
  assistantIds: [],
  window: '1d',
  since: '',
  until: '',
  last: DEFAULT_LAST,
  callId: '',
}

/** A `datetime-local` value as an RFC 3339 instant, or nothing if it is not a date yet.
 * A half-typed date must not go to the engine: `created_at >= "2026-0"` is a filter that
 * quietly matches the wrong calls rather than failing. */
function instant(local: string): string | null {
  if (!local) return null
  const t = new Date(local)
  return Number.isNaN(t.getTime()) ? null : t.toISOString()
}

export function toParams(f: Filters): URLSearchParams {
  const p = new URLSearchParams()
  if (f.org != null) p.set('org', String(f.org))
  // Repeated rather than joined: the engine reads several `assistant_id` as "any of these".
  for (const id of f.assistantIds) p.append('assistant_id', id)
  if (f.window) p.set('window', f.window)
  const since = instant(f.since)
  if (since) p.set('since', since)
  const until = instant(f.until)
  if (until) p.set('until', until)
  p.set('last', f.last.trim() || DEFAULT_LAST)
  if (f.callId.trim()) p.set('call_id', f.callId.trim())
  return p
}
