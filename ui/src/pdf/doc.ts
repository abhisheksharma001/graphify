// The page a PDF is written on: a cursor, a margin, and the half-dozen things anything
// downloaded from this dashboard is made of.
//
// jsPDF draws; it does not lay out. Every `text` call is an absolute coordinate, so
// without something like this file each report would be doing its own arithmetic and
// getting a different answer — one wrapping at a different width, another running off the
// bottom of the page because nothing counted the lines. So the arithmetic is here once and
// the three reports say what goes in.
//
// Two rules the whole file follows:
//
//   Nothing is drawn without first asking whether it fits. `need` is called before every
//   mark, so a table row is never half on one page and half on the next.
//
//   No value is formatted here. The numbers in a PDF are the numbers that were on screen,
//   which means they come through `format.ts` before they arrive — including the dash. A
//   report that rendered a missing cost as 0 while the table above it said "—" would be
//   the same bug the dashboard exists to avoid.

import { jsPDF } from 'jspdf'

/** A4 portrait, in millimetres, which is jsPDF's unit here. */
const PAGE = { w: 210, h: 297 }
const MARGIN = 15

/** The text column. Everything wraps to this. */
export const WIDTH = PAGE.w - MARGIN * 2

/** Where the body stops and the footer begins. */
const BOTTOM = PAGE.h - MARGIN - 6

/** The ink. Three greys and no colour: a chart carries the palette, the prose does not. */
const INK = '#0b0b0b'
const INK_2 = '#52514e'
const MUTED = '#898781'
const HAIRLINE = '#d8d7d0'

/** Leading, as a multiple of the font size, converted to millimetres. `pt` to `mm` is
 * 25.4/72; the 1.35 is the same loose leading the screen uses for body text. */
const line = (size: number) => (size * 25.4 * 1.35) / 72

/** The gap between two charts sharing a row. */
const COLUMN_GAP = 6

/** The gap under a table row. */
const ROW_GAP = 1.2

/** A label and its value. The value is already formatted — see the note above. */
export type Pair = [string, string]

/** One captured card: the image, the pixels it was taken at — only their ratio is read —
 * and whether it spanned the pack or sat in a column of it. */
export type Picture = { png: string; w: number; h: number; wide: boolean }

export type Column = {
  head: string
  /** Share of the width, relative to the other columns. */
  weight: number
  right?: boolean
}

export class Doc {
  private pdf: jsPDF
  /** The baseline the next thing is drawn from. */
  private y = MARGIN
  private stamp: string

  constructor() {
    this.pdf = new jsPDF({ unit: 'mm', format: 'a4' })
    this.stamp = new Date().toLocaleString()
  }

  /** Make room for `mm` of content, starting a page if there is not that much left.
   * Returns nothing: callers ask and then draw, they do not branch on the answer. */
  private need(mm: number): void {
    if (this.y + mm <= BOTTOM) return
    this.pdf.addPage()
    this.y = MARGIN
  }

  private write(text: string, size: number, colour: string, style = 'normal'): void {
    this.pdf.setFont('helvetica', style)
    this.pdf.setFontSize(size)
    this.pdf.setTextColor(colour)
    const lines = this.pdf.splitTextToSize(text, WIDTH) as string[]
    const step = line(size)
    for (const one of lines) {
      this.need(step)
      this.y += step
      this.pdf.text(one, MARGIN, this.y)
    }
  }

  gap(mm = 4): void {
    this.y += mm
  }

  /** The document's own title. Once, at the top of page one. */
  title(text: string): void {
    this.write(text, 20, INK, 'bold')
    this.gap(1)
  }

  heading(text: string): void {
    // A heading with nothing under it is a heading at the bottom of a page: reserve the
    // first line of whatever follows, so the two travel together.
    this.need(line(13) + line(10))
    this.gap(4)
    this.write(text, 13, INK, 'bold')
    this.gap(1)
  }

  para(text: string): void {
    this.write(text, 10, INK_2)
    this.gap(2)
  }

  note(text: string): void {
    this.write(text, 8.5, MUTED)
    this.gap(2)
  }

  /** Verbatim text — a rule, a criterion in the words it was typed in. Monospace, and no
   * wrapping cleverness: a long line is broken where it runs out of room, because a rule
   * re-flowed at word boundaries is a rule somebody could retype wrong. */
  code(text: string): void {
    this.pdf.setFont('courier', 'normal')
    this.pdf.setFontSize(8.5)
    this.pdf.setTextColor(INK_2)
    const step = line(8.5)
    for (const raw of text.split('\n')) {
      for (const one of this.pdf.splitTextToSize(raw, WIDTH) as string[]) {
        this.need(step)
        this.y += step
        this.pdf.text(one, MARGIN, this.y)
      }
    }
    this.gap(2)
  }

  /** Label-and-value rows, two to a line. What a report is about goes here: the org, the
   * window, the filters — the things that say which calls these numbers are. */
  pairs(rows: Pair[]): void {
    const half = WIDTH / 2
    const step = line(9)
    for (let i = 0; i < rows.length; i += 2) {
      this.need(step * 2)
      this.y += step
      rows.slice(i, i + 2).forEach(([label, value], n) => {
        const x = MARGIN + n * half
        this.pdf.setFont('helvetica', 'normal')
        this.pdf.setFontSize(7.5)
        this.pdf.setTextColor(MUTED)
        this.pdf.text(label.toUpperCase(), x, this.y)
        this.pdf.setFontSize(9)
        this.pdf.setTextColor(INK)
        const [first] = this.pdf.splitTextToSize(value, half - 4) as string[]
        this.pdf.text(first ?? '', x, this.y + step)
      })
      this.y += step
    }
    this.gap(3)
  }

  /** A table, with its header repeated on every page it runs onto. A row is drawn whole or
   * not at all, so a cell that wrapped to three lines takes its whole row to the next
   * page rather than leaving two of them behind. */
  table(columns: Column[], rows: string[][]): void {
    const total = columns.reduce((sum, c) => sum + c.weight, 0)
    const widths = columns.map((c) => (c.weight / total) * WIDTH)
    const x = widths.map((_, i) => MARGIN + widths.slice(0, i).reduce((a, b) => a + b, 0))
    const step = line(8.5)

    const header = () => {
      this.need(step * 2)
      this.y += step
      this.pdf.setFont('helvetica', 'bold')
      this.pdf.setFontSize(7.5)
      this.pdf.setTextColor(MUTED)
      columns.forEach((c, i) => {
        const at = c.right ? x[i] + widths[i] - 2 : x[i]
        this.pdf.text(c.head.toUpperCase(), at, this.y, c.right ? { align: 'right' } : undefined)
      })
      this.y += 1.5
      this.pdf.setDrawColor(HAIRLINE)
      this.pdf.line(MARGIN, this.y, MARGIN + WIDTH, this.y)
    }

    header()
    this.pdf.setFont('helvetica', 'normal')
    this.pdf.setFontSize(8.5)
    for (const row of rows) {
      const cells = row.map(
        (cell, i) => this.pdf.splitTextToSize(cell, widths[i] - 3) as string[],
      )
      const height = step * Math.max(...cells.map((c) => c.length))
      const before = this.y
      this.need(height + ROW_GAP)
      // A new page took the header with it, so redraw it before the row that caused it.
      if (this.y < before) {
        header()
        this.pdf.setFont('helvetica', 'normal')
        this.pdf.setFontSize(8.5)
      }
      this.pdf.setTextColor(INK_2)
      cells.forEach((cell, i) => {
        cell.forEach((one, n) => {
          const at = columns[i].right ? x[i] + widths[i] - 2 : x[i]
          const options = columns[i].right ? ({ align: 'right' } as const) : undefined
          this.pdf.text(one, at, this.y + step * (n + 1), options)
        })
      })
      // A breath between rows, so a cell that wrapped to two lines still reads as one row
      // rather than as two short ones.
      this.y += height + ROW_GAP
    }
    this.gap(3)
  }

  /** The charts, laid out the way the pack lays them out: a wide chart across the page,
   * and two narrow ones sharing a row.
   *
   * Not one picture per row. A card is captured at the width it was drawn at, so blowing a
   * half-width card up to fill the page magnifies its 15px heading to something the size of
   * this document's title — the same picture, and a lie about how much of the dashboard it
   * is. Drawing each card at the share of the width it had on screen keeps one scale across
   * the whole file, and puts the same charts beside each other as the screen does.
   */
  images(pictures: Picture[]): void {
    for (let i = 0; i < pictures.length; ) {
      const first = pictures[i]
      const next = pictures[i + 1]
      const second = !first.wide && next && !next.wide ? next : null
      const column = first.wide ? WIDTH : (WIDTH - COLUMN_GAP) / 2
      const pair = second === null ? [first] : [first, second]
      const heights = pair.map((p) => column * (p.h / p.w))

      // A card taller than a page body is drawn smaller rather than split. Both of a pair
      // shrink by the same factor: two charts side by side at two scales would be a chart
      // and a claim about it.
      const scale = Math.min(1, (BOTTOM - MARGIN) / Math.max(...heights))
      const width = column * scale
      const height = Math.max(...heights) * scale

      this.need(height + 2)
      this.y += 2
      pair.forEach((p, n) => {
        // Deflated, not stored. A chart is flat colour and straight lines, the case PNG's
        // filter was built for: without this a full dashboard is twenty megabytes, and
        // with it half of one.
        this.pdf.addImage(
          p.png,
          'PNG',
          MARGIN + n * (width + COLUMN_GAP),
          this.y,
          width,
          heights[n] * scale,
          undefined,
          'FAST',
        )
      })
      this.y += height
      this.gap(2)
      i += pair.length
    }
  }

  /** Page numbers and the moment it was made, on every page, then download it.
   *
   * The stamp matters more than it looks: a dashboard is a window over live data, so a
   * page of it with no time on it is a claim about now that will still be lying next
   * week. */
  save(name: string): void {
    const pages = this.pdf.getNumberOfPages()
    for (let n = 1; n <= pages; n++) {
      this.pdf.setPage(n)
      this.pdf.setFont('helvetica', 'normal')
      this.pdf.setFontSize(7.5)
      this.pdf.setTextColor(MUTED)
      this.pdf.text(`graphify · ${this.stamp}`, MARGIN, PAGE.h - MARGIN + 2)
      this.pdf.text(`${n} / ${pages}`, MARGIN + WIDTH, PAGE.h - MARGIN + 2, { align: 'right' })
    }
    this.pdf.save(name)
  }
}

/** What a downloaded file is called: what it is, and the day it was taken.
 *
 * The reader's day, not UTC's. The stamp inside the file is local, and a file named for
 * yesterday carrying a footer that says today is a file somebody will file wrong. */
export function filename(what: string): string {
  const now = new Date()
  const pad = (n: number) => String(n).padStart(2, '0')
  const day = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
  return `graphify-${what}-${day}.pdf`
}
