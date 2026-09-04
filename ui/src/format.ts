// How a value becomes text.
//
// Every formatter here goes through one rule first: a missing number is drawn as "—" and
// never as a zero. An hour that priced nothing has no cost; that is a different fact from
// a cost of nothing, and the difference is the whole point of the dashboard.

export const hour = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
})
export const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })
export const full = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short',
})

export const DASH = '—'

export function money(v: number | null): string {
  return v === null ? DASH : `$${v.toFixed(2)}`
}

export function count(v: number | null): string {
  return v === null ? DASH : String(v)
}

export function millis(v: number | null): string {
  return v === null ? DASH : `${Math.round(v)} ms`
}

/** Seconds as m:ss, because a call is minutes long and 143 is harder to read than 2:23. */
export function seconds(v: number | null): string {
  if (v === null) return DASH
  const total = Math.round(v)
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

export function tokens(v: number | null): string {
  if (v === null) return DASH
  return v >= 1000 ? `${(v / 1000).toFixed(1)}k` : String(v)
}

/** An average of a number the dashboard knows nothing else about — no unit, no scale. Two
 * decimals at most, and no trailing zeroes, so 4 reads as 4 and 4.5 as 4.5. */
export function mean(v: number | null): string {
  return v === null ? DASH : String(Math.round(v * 100) / 100)
}

/** A `boolean | null` as text. `false` is an answer and says so; only NULL is a dash. */
export const yesNo = (v: boolean | null) => (v === null ? DASH : v ? 'yes' : 'no')

/** Tool calls and how many of them failed, as one fact. A call that made no tool calls
 * made no failed ones either, so the failure count is only worth its own words when there
 * is one.
 *
 * Here rather than in the table, because a downloaded call list is the table: two spellings
 * of "3 · 1 failed" would be two documents disagreeing about one call. */
export function tools(calls: number | null, failures: number | null): string {
  if (calls === null) return DASH
  const failed = failures ?? 0
  return failed > 0 ? `${calls} · ${failed} failed` : String(calls)
}

/** What a pattern is called when it was saved without a name: its id, because that is what
 * the engine calls it in every message about it. Written once, so the list, the heading and
 * the chart title can never come to disagree about which pattern is which. */
export const named = (id: number, name: string | null) => name?.trim() || `#${id}`
