// A structured key whose values are objects or lists.
//
// There is no count of those and no average of them, so the card says what is there rather
// than drawing something that is not. Leaving the key out altogether would say the data is
// absent when it is present, which is the same lie as drawing a missing number as zero.

import type { StructuredField } from '../api'

export default function NotChartable({
  field,
  stale,
}: {
  field: StructuredField
  stale: boolean
}) {
  return (
    <section className="card">
      <h2>{field.key}</h2>
      <div className={stale ? 'stale' : undefined}>
        <p className="notice">
          {field.calls} calls carried this key. Its values are objects or lists, which are
          neither counts nor numbers — open a call to read them.
        </p>
      </div>
    </section>
  )
}
