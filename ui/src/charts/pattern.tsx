// The one chart a saved pattern gets: how many calls of the selection it matched, bucket
// by bucket.
//
// Nothing here counts anything. `/api/stats` was asked with `pattern=` on it, so `calls`
// on every bucket below was counted by the engine over exactly the rows in the table
// underneath — the chart and its table cannot come to describe different calls.
//
// The kind is the brain's suggestion from the day it wrote the rule. `Line` is a trend and
// `Bar` is a tally; a suggestion this dashboard does not have is drawn as a line, because
// a pattern is nearly always a question about a direction.

import type { Bucket } from '../api'
import { count } from '../format'
import Bucketed from './Bucketed'
import type { Measure } from './Bucketed'

/** One series, so it wears slot one. Numbering it by position would mean the colour moved
 * when the list beside it did, and colour follows the entity, never its rank. */
const ONE = 'var(--s-1)'

/** BAML spells them `Line` and `Bar`. Compared in lower case so a rename of the enum's
 * casing cannot silently turn every pattern's chart into the fallback. */
const isBar = (kind: string | undefined) => kind?.toLowerCase() === 'bar'

export default function PatternChart({
  title,
  sub,
  buckets,
  bucketSize,
  kind,
  stale,
}: {
  title: string
  sub: string
  buckets: Bucket[]
  bucketSize: string
  kind: string | undefined
  stale: boolean
}) {
  // A bucket that matched nothing is a real zero — the calls were read and none of them
  // matched — so this is the one number on the dashboard that is not drawn as a gap.
  const matched: Measure = { field: 'calls', label: 'matched calls', colour: ONE }
  const bar = isBar(kind)
  return (
    <Bucketed
      title={title}
      sub={sub}
      buckets={buckets}
      bucketSize={bucketSize}
      stale={stale}
      stack={bar ? [matched] : []}
      lines={bar ? [] : [matched]}
      format={count}
      empty="No call in this selection matched this pattern."
    />
  )
}
