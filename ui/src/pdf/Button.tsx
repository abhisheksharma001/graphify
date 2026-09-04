// The button every download hangs off.
//
// Making a PDF is slow enough to notice — the dashboard's captures are a repaint of the
// page per chart — so the button says so and refuses to be pressed twice. A second press
// would not produce a second file any faster; it would produce two.

import { useState } from 'react'

export default function PdfButton({
  label = 'Download PDF',
  make,
  onError,
}: {
  label?: string
  /** The work. Sync or async — both are awaited, so a throw either way lands in `onError`
   * rather than in the console. */
  make: () => void | Promise<void>
  onError: (e: unknown) => void
}) {
  const [busy, setBusy] = useState(false)
  return (
    <button
      type="button"
      className="pdf-button"
      disabled={busy}
      onClick={() => {
        setBusy(true)
        Promise.resolve()
          .then(make)
          .catch(onError)
          .finally(() => setBusy(false))
      }}
    >
      {busy ? 'Preparing…' : label}
    </button>
  )
}
