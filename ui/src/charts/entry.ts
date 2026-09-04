// What a card on the dashboard is, before anything has decided to draw it.
//
// Its own file, and not part of the chart chrome, because this is the one thing the
// dashboard and the charts have to agree on: the charts say what the cards are, the
// dashboard says which of them appear and in what order, and neither needs to know
// anything else about the other.

import type { ReactNode } from 'react'

/** One card, and the id the saved layout knows it by.
 *
 * The id is what persists, so it is written down rather than derived from the title:
 * rewording a chart must not turn it back on for everyone who had turned it off. */
export type Entry = {
  id: string
  title: string
  /** A chart that reads across the whole span rather than across one measure of it, and
   * so spans the pack instead of sitting in a column of it. */
  wide?: boolean
  node: ReactNode
}

/** One entry, with its title written once: the menu shows it and the card's own heading is
 * it, so the two can never come to disagree about what a chart is called. */
export function card(id: string, title: string, node: (title: string) => ReactNode): Entry {
  return { id, title, node: node(title) }
}
