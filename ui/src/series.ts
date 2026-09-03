// What the ended-group chart draws, assembled from two requests.
//
// `/api/stats` gives the axis: the bucket size for this span, every bucket across it
// including the empty ones, and each bucket's cost. What it cannot give is a breakdown of
// a bucket by ended group — it counts groups over the whole selection, not per bucket.
//
// So the counts come from `/api/calls`, which returns the same selection row by row, each
// carrying its group and its raw reason. Bucketing those in the browser is exact, needs
// one request rather than one per group, and is the only way to put the raw reasons *for
// a bucket* in that bucket's tooltip.
//
// This is why the filter bar always sends a `last`: `/api/calls` is a page, and a page
// with no size is a page of 200. With `last` set both endpoints select the same calls —
// same filters, same newest-first order, same cut — so the axis and the bars agree.

import * as api from './api'
import { display, STACK } from './groups'
import type { Group } from './groups'

const HOUR_MS = 3_600_000
const DAY_MS = 86_400_000

/** The label for a call that ended for no reason the engine was told about. */
export const NO_REASON = '—'

export type Series = {
  group: Group
  total: number
  /** Bucket start (epoch ms) → calls. Absent means none, which is a real zero. */
  counts: Map<number, number>
  /** Bucket start → raw `endedReason` → calls, for that bucket's tooltip. */
  reasons: Map<number, Map<string, number>>
}

export type Chart = {
  /** The x axis: every bucket across the span, in order, with the bucket's cost. Cost is
   * null when nothing in the bucket was priced — which is not a cost of zero. */
  buckets: { ms: number; cost: number | null }[]
  bucketSize: string
  series: Series[]
  calls: number
  /** Calls with no `createdAt`. They count, but they cannot go on a time axis. */
  undated: number
  /** True when `last` is what ended the selection, so the chart is a page and says so. */
  capped: boolean
}

export async function load(params: URLSearchParams): Promise<Chart> {
  const [stats, calls] = await Promise.all([api.stats(params), api.calls(params)])
  const step = stats.bucket_size === '1h' ? HOUR_MS : DAY_MS

  // Both bucket rules are exact divisors of the epoch, so flooring lands on the same
  // instants the engine truncates to — no string arithmetic on its bucket labels.
  const cost = new Map<number, number | null>()
  for (const b of stats.per_bucket) cost.set(Date.parse(b.bucket), b.cost)

  const byGroup = new Map<Group, Series>()
  let undated = 0
  for (const call of calls) {
    // A NULL group is a call that has not ended, which the engine also calls "unknown".
    const group = display(call.ended_group ?? 'unknown')
    const series =
      byGroup.get(group) ??
      { group, total: 0, counts: new Map(), reasons: new Map() }
    byGroup.set(group, series)
    series.total += 1

    const at = call.created_at ? Date.parse(call.created_at) : NaN
    if (Number.isNaN(at)) {
      undated += 1
      continue
    }
    const ms = Math.floor(at / step) * step
    series.counts.set(ms, (series.counts.get(ms) ?? 0) + 1)
    const reasons = series.reasons.get(ms) ?? new Map<string, number>()
    series.reasons.set(ms, reasons)
    const reason = call.ended_reason ?? NO_REASON
    reasons.set(reason, (reasons.get(reason) ?? 0) + 1)
    if (!cost.has(ms)) cost.set(ms, null)
  }

  const buckets = [...cost.entries()]
    .map(([ms, c]) => ({ ms, cost: c }))
    .sort((a, b) => a.ms - b.ms)

  return {
    buckets,
    bucketSize: stats.bucket_size,
    // Fixed order, so a filter that removes a group never repaints the ones that stay.
    series: STACK.map((g) => byGroup.get(g)).filter((s) => s !== undefined),
    calls: calls.length,
    undated,
    capped: calls.length > 0 && calls.length === Number(params.get('last')),
  }
}
