// The call list, downloaded.
//
// The rows the table is showing, in its order, through the same formatters — so a cost the
// screen wrote as "—" is a dash here too. That is the point of the file: a PDF that
// rendered a call Vapi never priced as $0.00 would be a document making a claim the
// dashboard refuses to make.

import type { Call } from '../api'
import { DASH, full, money, seconds, tools, yesNo } from '../format'
import { Doc, filename } from './doc'
import type { Pair } from './doc'

/** The table's own columns, in the table's own order. */
const COLUMNS = [
  { head: 'Started', weight: 2.4 },
  { head: 'Assistant', weight: 2.2 },
  { head: 'Duration', weight: 1.1, right: true },
  { head: 'Ended', weight: 2.6 },
  { head: 'Tools', weight: 1.4 },
  { head: 'Transferred', weight: 1.3 },
  { head: 'Cost', weight: 1.1, right: true },
]

const row = (c: Call): string[] => [
  c.created_at ? full.format(new Date(c.created_at)) : DASH,
  c.assistant_name ?? c.assistant_id ?? DASH,
  seconds(c.duration_s),
  c.ended_reason ?? c.ended_group ?? DASH,
  tools(c.tool_calls, c.tool_failures),
  yesNo(c.transferred),
  money(c.cost),
]

export function callsPdf(selection: Pair[], rows: Call[]): void {
  const doc = new Doc()
  doc.title('Calls')
  doc.pairs(selection)
  doc.note(
    `${rows.length} call${rows.length === 1 ? '' : 's'} in this selection, newest first. ` +
      'A dash is a value nobody recorded, and is not a zero.',
  )
  if (rows.length === 0) doc.para('No calls in this selection.')
  else doc.table(COLUMNS, rows.map(row))
  doc.save(filename('calls'))
}
