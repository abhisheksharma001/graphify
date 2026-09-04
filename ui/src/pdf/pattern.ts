// One pattern, downloaded: what it counts, how it was learned, and the calls it found.
//
// A pattern is an argument — the criterion is the claim, the plan is what a model
// understood by it, the rule is what actually runs every day for nothing, and the
// agreement is how far apart those last two were on the calls the model read. All four go
// in the file, because a page listing matched calls without them is a number nobody can
// check.
//
// The matched calls carry their evidence: the sentence the model gave for saying this call
// is one of them. Only the sample was ever read, so most matched calls have no evidence at
// all, and that is a dash rather than an empty cell.

import type { Call, Pattern } from '../api'
import { DASH, count, full, named } from '../format'
import { Doc, filename } from './doc'
import type { Pair } from './doc'

/** How many calls the file lists. The whole matched set can be thousands; twenty is enough
 * to see what the rule is actually catching, which is what a reader opens this for. */
const CALLS = 20

const percent = (x: number | null) => (x === null ? DASH : `${Math.round(x * 100)}%`)

const CALL_COLUMNS = [
  { head: 'Started', weight: 2.2 },
  { head: 'Assistant', weight: 1.9 },
  { head: 'Ended', weight: 2.5 },
  { head: 'Evidence', weight: 4.8 },
]

export function patternPdf(selection: Pair[], pattern: Pattern, calls: Call[]): void {
  const doc = new Doc()
  doc.title(named(pattern.id, pattern.name))
  doc.pairs([
    ...selection,
    ['Matched here', count(pattern.matched)],
    ['Mode', pattern.mode ?? 'free'],
    ['Agreement', percent(pattern.agreement)],
    ['Sample', pattern.sample_size === null ? DASH : `${pattern.sample_size} calls`],
    ['Learned by', pattern.model ?? DASH],
    ['Learned on', pattern.created_at ? full.format(new Date(pattern.created_at)) : DASH],
  ])

  doc.heading('Criterion')
  doc.para(pattern.criterion?.trim() || 'Saved without one.')

  doc.heading('The plan')
  if (pattern.plan === null || pattern.plan.rows.length === 0) {
    doc.para('No plan was stored with this pattern.')
  } else {
    doc.table(
      [
        { head: 'If', weight: 1 },
        { head: 'Then', weight: 1 },
      ],
      pattern.plan.rows.map((r) => [r.if_, r.then]),
    )
    if (pattern.plan.reason) doc.para(pattern.plan.reason)
    doc.note(
      `Confidence ${percent(pattern.plan.confidence)} · a rule can check it: ` +
        `${pattern.plan.expressible ? 'yes' : 'no'}`,
    )
  }

  doc.heading('The rule')
  doc.note('What runs over every call, every day, without a model and without a cost.')
  doc.code(pattern.rule == null ? 'none' : JSON.stringify(pattern.rule, null, 2))

  doc.heading('Matched calls')
  const shown = calls.slice(0, CALLS)
  doc.note(
    shown.length === 0
      ? 'None of the calls in this selection.'
      : `${shown.length} of ${count(pattern.matched)} matched here. A call with no evidence ` +
          'is one the model never read — only the sample was labelled.',
  )
  if (shown.length > 0) {
    doc.table(
      CALL_COLUMNS,
      shown.map((c) => [
        c.created_at ? full.format(new Date(c.created_at)) : DASH,
        c.assistant_name ?? c.assistant_id ?? DASH,
        c.ended_reason ?? c.ended_group ?? DASH,
        c.evidence?.trim() || DASH,
      ]),
    )
  }
  doc.save(filename(`pattern-${pattern.id}`))
}
