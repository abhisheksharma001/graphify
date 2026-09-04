// The dashboard, downloaded.
//
// One picture per chart that is drawn, in the order the reader put them in and at the
// width they were drawn at — the same order and the same shape, because the pictures come
// from the cards themselves rather than from a second list this file keeps. A chart turned
// off is not in the PDF for the same reason it is not on the screen, and neither this file
// nor `Dashboard` has to know which charts exist.

import { shots } from './capture'
import { Doc, filename } from './doc'
import type { Pair } from './doc'

/** A card on the page, and whether it spans the pack. The flag comes from the chart's own
 * entry, not from measuring the picture: a card that happens to be square is not a wide
 * one, and only the entry knows which it is. */
export type Card = { node: HTMLElement; wide: boolean }

export async function dashboardPdf(selection: Pair[], cards: Card[]): Promise<void> {
  // Photograph first. Every capture is a repaint of the page, and doing them before a
  // single line is written keeps the whole flicker in one place.
  const pictures = await shots(cards.map((c) => c.node))

  const doc = new Doc()
  doc.title('Dashboard')
  doc.pairs(selection)
  doc.note(
    'Every chart below was drawn over this selection. A dash is a value nobody recorded, ' +
      'and is not a zero.',
  )
  if (pictures.length === 0) {
    doc.para('No charts are turned on, so there is nothing here to show.')
  }
  doc.images(pictures.map((shot, i) => ({ ...shot, wide: cards[i].wide })))
  doc.save(filename('dashboard'))
}
