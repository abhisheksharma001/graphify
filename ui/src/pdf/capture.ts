// Turning what is on screen into a picture a PDF can hold.
//
// A chart here is SVG that gets its colours from CSS variables, so a capture is whatever
// the reader's theme was at the moment it was taken. On a dark screen that is white ink on
// near-black, which prints as a black slab and reads as nothing at all on the projector
// somebody puts it on.
//
// So the theme is pinned to light for the length of the capture and put back afterwards.
// The stylesheet already supports being told: it declares the dark steps twice, once for
// the OS setting and once for an explicit `data-theme`, precisely so an explicit choice
// wins in both directions. This is that choice, held for two frames.

import { toPng } from 'html-to-image'

/** A captured card: the image, and the pixels it was taken at. Only the ratio is read, so
 * a capture at two device pixels per CSS pixel is the same picture, sharper. */
export type Shot = { png: string; w: number; h: number }

/** Twice the CSS pixels. A chart's axis labels are 11px, and at one-to-one they come out
 * of a 210mm-wide page soft enough to argue with. */
const SCALE = 2

/** Chrome, not data. The table toggle is a control — it does nothing in a PDF and would
 * sit under every chart looking like a caption. */
const isControl = (node: HTMLElement) =>
  node.classList?.contains('table-toggle') || node.classList?.contains('pdf-button')

/** Photograph these nodes, in order, in light mode.
 *
 * One pass over all of them rather than one call per chart: flipping the theme is a
 * repaint of the whole page, and doing it once per card would make the screen strobe.
 */
export async function shots(nodes: HTMLElement[]): Promise<Shot[]> {
  const root = document.documentElement
  const was = root.dataset.theme
  root.dataset.theme = 'light'
  // Two frames: one for the style change to be applied, one for it to have been painted.
  // Without the wait the capture reads the variables that were in force a moment ago.
  await frame()
  await frame()
  try {
    const surface = getComputedStyle(root).getPropertyValue('--surface').trim()
    const out: Shot[] = []
    for (const node of nodes) {
      const box = node.getBoundingClientRect()
      const png = await toPng(node, {
        pixelRatio: SCALE,
        backgroundColor: surface || '#ffffff',
        // Nothing here loads a webfont, and asking html-to-image to inline the ones the
        // system supplies means fetching stylesheets it cannot read and waiting for the
        // failures. The PDF sets its own fonts anyway.
        skipFonts: true,
        filter: (n) => !(n instanceof HTMLElement && isControl(n)),
      })
      out.push({ png, w: box.width, h: box.height })
    }
    return out
  } finally {
    // Whatever happened, the reader gets their screen back. `delete` and not `= ''`: an
    // empty `data-theme` is still an attribute, and it matches neither of the two
    // selectors the stylesheet writes, which would leave the page stuck in light.
    if (was === undefined) delete root.dataset.theme
    else root.dataset.theme = was
  }
}

const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()))
