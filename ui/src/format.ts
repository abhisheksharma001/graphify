// How a number becomes text.
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
