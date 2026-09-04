// What Vapi's own analysis said about the calls.
//
// Two things come back from it: `successEvaluation`, which is one string per call, and
// `structuredData`, which is whatever schema the assistant was configured with. The first
// is a known chart. The second is not — the keys are the user's, not ours, so the engine
// classifies each one by the values it actually carried and this file draws the one chart
// that key can honestly carry.
//
// Which means these cards come and go with the data: a key nothing carries has no card at
// all. Their ids are namespaced under `structured:` so a schema with a key called "cost"
// in it can never be mistaken for the cost chart in the saved layout.
//
// Nothing here spends a model token. Every number was extracted at sync time and has been
// sitting in the database since; this is a read.

import type { Stats, StructuredField } from '../api'
import { mean } from '../format'
import Bucketed from './Bucketed'
import { card } from './entry'
import type { Entry } from './entry'
import NotChartable from './NotChartable'
import Ranked from './Ranked'
import type { Item } from './Ranked'

/** Every chart below draws one series, and each sits on a card that names it. So they all
 * wear slot one: numbering them by their position in the list would mean a key appearing
 * or disappearing repainted its neighbours, and colour must never follow order. */
const ONE = 'var(--s-1)'

/** What a structured key's chart is called in the saved layout. */
export const STRUCTURED = 'structured:'

/** The values, plus the row the engine folded. The fold was summed over every value, not
 * over the ten that fitted, so drawing it keeps the chart adding up to the calls. */
function items(field: StructuredField): Item[] {
  const rows: Item[] = Object.entries(field.counts).map(([name, value]) => ({ name, value }))
  if (field.tail) {
    rows.push({ name: 'other', value: field.tail.calls, note: `${field.tail.values} more` })
  }
  return rows
}

function chartFor(field: StructuredField, bucketSize: string, stale: boolean): Entry {
  return card(`${STRUCTURED}${field.key}`, field.key, (title) => {
    if (field.kind === 'number') {
      return (
        <Bucketed
          title={title}
          sub={`${field.calls} calls carried a number. Each bucket averages the ones in it.`}
          buckets={field.per_bucket}
          bucketSize={bucketSize}
          stale={stale}
          lines={[{ field: 'avg', label: 'average', colour: ONE }]}
          format={mean}
        />
      )
    }
    if (field.kind === 'text') {
      return (
        <Ranked
          title={title}
          sub={`${field.calls} calls carried this key.`}
          items={items(field)}
          unit="calls"
          stale={stale}
          empty="No call carried this key."
        />
      )
    }
    return <NotChartable field={field} stale={stale} />
  })
}

export default function analysisEntries(stats: Stats, stale: boolean): Entry[] {
  const evaluations: Item[] = Object.entries(stats.success_eval_counts).map(
    ([name, value]) => ({ name, value }),
  )

  return [
    /* No colour verdict on the bars. `successEvaluation` is whatever rubric the assistant
     * was given — "true", "8", a sentence — and painting a guess at which of those is good
     * would be the dashboard inventing an opinion it does not have. */
    card('success_eval', 'Success evaluation', (title) => (
      <Ranked
        title={title}
        sub="Vapi's own verdict, counted. A call it did not evaluate is not in here."
        items={evaluations}
        unit="calls"
        stale={stale}
        empty="Vapi ran no success evaluation on these calls."
      />
    )),

    ...stats.structured_fields.map((f) => chartFor(f, stats.bucket_size, stale)),
  ]
}
