// Ended groups: which ones get their own colour, and what happens to the rest.
//
// The engine sorts every Vapi `endedReason` into one of eleven groups. The palette stops
// at eight hues, and a ninth generated hue is indistinguishable from one of the eight
// under colourblindness, so three groups fold into a grey residual bucket instead. The
// fold is fixed, never data-driven: a filter that changes which groups are on screen must
// not repaint the ones that survive.
//
// Nothing is hidden by folding. The residual bucket is named in the legend, counted in
// the table, and hovering it lists the raw reasons it covers.

/** The eight that carry their own hue, bottom of the stack to top. This order is also
 * the palette order, and the palette order is the colourblind-safety mechanism — it was
 * chosen by running the data-viz validator over candidate orderings and keeping one that
 * clears every adjacent-pair gate in both light and dark. Reordering means re-running it. */
export const NAMED = [
  'customer',
  'assistant',
  'unknown',
  'timeout',
  'transfer-error',
  'stt-error',
  'tts-error',
  'llm-error',
] as const

/** Where `transport`, `start-error`, `other`, and anything the engine grows later land. */
export const OTHER = 'other'

export type Group = (typeof NAMED)[number] | typeof OTHER

/** The stack, bottom to top. The residual sits on top, where a reader expects a tail. */
export const STACK: Group[] = [...NAMED, OTHER]

const named = new Set<string>(NAMED)

/** Which display group an engine group is drawn as. */
export function display(group: string): Group {
  return named.has(group) ? (group as Group) : OTHER
}

export function colour(group: Group): string {
  return `var(--g-${group})`
}
