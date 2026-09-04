// The seam between the three buttons and the two libraries behind them.
//
// jsPDF and html-to-image are together about 350 kB — a third again on top of everything
// else this page loads, for a button most readers will never press. So nothing above this
// file imports them: the three functions here are the same three signatures, and each one
// fetches its report the moment somebody actually asks for a file.
//
// The types cross the boundary anyway. `import type` is erased, so the shapes are checked
// at build time without any of the code arriving at load time.

import type { Call, Pattern } from '../api'
import type { Card } from './dashboard'
import type { Pair } from './doc'

export type { Card, Pair }

export async function dashboardPdf(selection: Pair[], cards: Card[]): Promise<void> {
  const { dashboardPdf: make } = await import('./dashboard')
  return make(selection, cards)
}

export async function callsPdf(selection: Pair[], rows: Call[]): Promise<void> {
  const { callsPdf: make } = await import('./calls')
  return make(selection, rows)
}

export async function patternPdf(
  selection: Pair[],
  pattern: Pattern,
  calls: Call[],
): Promise<void> {
  const { patternPdf: make } = await import('./pattern')
  return make(selection, pattern, calls)
}
