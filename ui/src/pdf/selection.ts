// Which calls a report is about, in words.
//
// This is the part of a PDF that makes it worth having. A page of charts with no header is
// a picture of some numbers; the same page over "acme · every assistant · the last day ·
// newest 250" is a document somebody can be shown a week later and still argue with.
//
// The filter bar is already the one source of truth for the slice, so this reads it and
// nothing else. Nothing here goes back to the engine: a report describes the selection the
// numbers in it were taken over, and asking again could describe a different one.

import type { Assistant, Org } from '../api'
import type { Filters } from '../filters'
import { DASH, full } from '../format'
import type { Pair } from './doc'

/** A `datetime-local` value as words, or the dash if it is not a date yet. */
const at = (local: string) => {
  const t = new Date(local)
  return Number.isNaN(t.getTime()) ? DASH : full.format(t)
}

/** The span, however it was chosen. A preset and a custom range are the same fact told two
 * ways, so they get one row rather than two, and only one of them is ever set. */
function span(f: Filters): string {
  if (f.window) return `the last ${f.window}`
  if (!f.since && !f.until) return 'everything stored'
  if (f.since && f.until) return `${at(f.since)} to ${at(f.until)}`
  return f.since ? `since ${at(f.since)}` : `until ${at(f.until)}`
}

/** Whose calls, by name. An empty pick is every assistant — which is a choice, not a
 * missing value, so it says so rather than showing a dash. */
function whose(f: Filters, assistants: Assistant[]): string {
  if (f.assistantIds.length === 0) return 'every assistant'
  const names = new Map(assistants.map((a) => [a.id, a.name?.trim() || a.id]))
  return f.assistantIds.map((id) => names.get(id) ?? id).join(', ')
}

/** The header rows of every report. `Call ID` only appears when there is one: a row
 * reading "Call ID —" would suggest the filter exists and is empty, which is a different
 * thing from it not being in force. */
export function describe(f: Filters, orgs: Org[], assistants: Assistant[]): Pair[] {
  const org = orgs.find((o) => o.id === f.org)
  const rows: Pair[] = [
    ['Org', org?.name ?? DASH],
    ['Assistants', whose(f, assistants)],
    ['Window', span(f)],
    ['Newest', `${f.last.trim() || DASH} calls`],
  ]
  if (f.callId.trim()) rows.push(['Call ID', f.callId.trim()])
  return rows
}
