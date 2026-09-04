// The rest of the dashboard: everything `/api/stats` answers directly.
//
// Unlike the ended-group stack, none of these needs the call list. Each is a field the
// engine already aggregates over the selection the filter bar chose, so the whole pack is
// drawn from the one `/api/stats` response the loader already has.
//
// Where a chart stacks components under a total, the components are what the total is
// made of — Vapi bills the cost slices separately and they add up to `cost`, and it
// reports the latency components separately and they add up to the average turn. Nothing
// here puts two different scales on one chart.

import type { Stats } from '../api'
import Bucketed from './Bucketed'
import type { Measure } from './Bucketed'
import Ranked from './Ranked'
import type { Item } from './Ranked'
import { count, millis, money, seconds, tokens } from '../format'

/** Cost, bottom of the stack to top. Slots from the series palette in a fixed order, so a
 * selection with no analysis cost never repaints the slices that are there. */
const COST: Measure[] = [
  { field: 'cost_llm', label: 'llm', colour: 'var(--s-1)' },
  { field: 'cost_tts', label: 'tts', colour: 'var(--s-2)' },
  { field: 'cost_stt', label: 'stt', colour: 'var(--s-3)' },
  { field: 'cost_vapi', label: 'vapi', colour: 'var(--s-4)' },
  { field: 'cost_transport', label: 'transport', colour: 'var(--s-5)' },
  { field: 'cost_analysis', label: 'analysis', colour: 'var(--s-6)' },
]

const TOKENS: Measure[] = [
  { field: 'prompt_tokens', label: 'prompt', colour: 'var(--s-1)' },
  { field: 'completion_tokens', label: 'completion', colour: 'var(--s-2)' },
  { field: 'cached_tokens', label: 'cached', colour: 'var(--s-3)' },
]

/** The four things a turn is spent waiting on, in the order they happen: the caller stops
 * talking, the transcriber finishes, the model answers, the voice speaks. */
const LATENCY: Measure[] = [
  { field: 'latency_transcriber', label: 'transcriber', colour: 'var(--s-1)' },
  { field: 'latency_endpointing', label: 'endpointing', colour: 'var(--s-2)' },
  { field: 'latency_model', label: 'model', colour: 'var(--s-3)' },
  { field: 'latency_voice', label: 'voice', colour: 'var(--s-4)' },
]

/** Drawn over the stack, in the same milliseconds. They are percentiles of the turn, not
 * of any component, so they sit above the bars rather than inside them. */
const PERCENTILES: Measure[] = [
  { field: 'latency_p50', label: 'p50', colour: 'var(--s-6)' },
  { field: 'latency_p95', label: 'p95', colour: 'var(--s-7)' },
]

export default function Pack({ stats, stale }: { stats: Stats; stale: boolean }) {
  const buckets = stats.per_bucket
  const size = stats.bucket_size

  const failures: Item[] = Object.entries(stats.tool_failures_by_name).map(
    ([name, value]) => ({ name, value }),
  )

  // An assistant the engine has never fetched a name for is still a real assistant with
  // real calls; it is labelled by the id it does have, not hidden.
  const assistants: Item[] = stats.by_assistant.map((a) => ({
    name: a.name ?? a.assistant_id ?? 'no assistant',
    value: a.calls,
  }))

  return (
    <div className="pack">
      <Ranked
        title="Tool failures by tool"
        sub="Failed tool calls across the selection."
        items={failures}
        unit="failures"
        stale={stale}
        empty="No tool call failed in this range."
      />

      <Ranked
        title="Calls per assistant"
        items={assistants}
        unit="calls"
        stale={stale}
        empty="No calls in this range."
      />

      <Bucketed
        title="Tool failures over time"
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        lines={[{ field: 'tool_failures', label: 'failures', colour: 'var(--s-1)' }]}
        format={count}
      />

      <Bucketed
        title="Transfers over time"
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        lines={[{ field: 'transfers', label: 'transfers', colour: 'var(--s-2)' }]}
        format={count}
      />

      <Bucketed
        title="Cost"
        sub="What each bucket was billed for. The slices add up to its cost."
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        stack={COST}
        format={money}
        /* A selection can have a cost and no breakdown of it — the total is one field and
         * the slices are six others. Saying so is the difference between "this cost
         * nothing" and "Vapi did not say what this was spent on". */
        empty="Vapi reported no cost breakdown for these calls."

      />

      <Bucketed
        title="Turn latency"
        sub="Bars are the average turn, split by what it waited on; the lines are the p50 and p95 of the turn itself."
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        stack={LATENCY}
        lines={PERCENTILES}
        format={millis}
      />

      <Bucketed
        title="Tokens"
        sub="Summed over the calls in each bucket."
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        stack={TOKENS}
        format={tokens}
      />

      <Bucketed
        title="Call duration"
        sub="Averaged over the calls that ended. One still running has no duration to average in."
        buckets={buckets}
        bucketSize={size}
        stale={stale}
        lines={[{ field: 'duration_avg', label: 'average', colour: 'var(--s-4)' }]}
        format={seconds}
      />
    </div>
  )
}
