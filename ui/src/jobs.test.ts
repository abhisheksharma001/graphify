// The headline on a failed job.
//
// Two sources and the split between them is the thing under test. An ending the engine
// wrote carries the engine's own sentence on the row, and that is the headline. An ending
// the brain wrote carries none, and the headline is picked out of whatever the brain wrote
// to stderr by a regex its own comment calls "a guess, and a load-bearing one" — which is
// exactly the kind of thing worth holding: the guess is right for the two shapes the brain
// actually produces and there is no type that says so.

import { afterEach, describe, expect, test, vi } from 'vitest'

import { Cancelled, JobFailed, settle } from './jobs'
import type { Job } from './api'

const failed = (log: string): Job => ({
  id: 1,
  kind: 'plan',
  status: 'failed',
  progress: null,
  estimate_usd: null,
  cost_usd: null,
  output: null,
  // The brain's own ending: the engine had nothing to add, so the headline is the guess.
  note: null,
  log,
  created_at: '2026-01-01T00:00:00Z',
  finished_at: '2026-01-01T00:00:01Z',
})

describe('what a failed job says it was', () => {
  test('the brain refusing is quoted whole', () => {
    // The brain's own refusals are one tidy line with no exception name, and the whole of
    // that line is the answer. This is the shape S-33 and S-34 added most of.
    const job = failed('plan: this message could cost up to $0.1094, over the $0.0500 cap\n')

    expect(new JobFailed(job).message).toBe(
      'plan: this message could cost up to $0.1094, over the $0.0500 cap',
    )
  })

  test('a traceback is reported by the line that names the fault, not by its last line', () => {
    // The whole reason the regex exists. The last line of this traceback is true and tells
    // nobody anything; the answer is four lines above it.
    const job = failed(
      [
        'Traceback (most recent call last):',
        '  File "/app/graphify_brain/plan.py", line 130, in plan',
        '    result = client().with_options(',
        "BamlError: LLM client 'Sonnet' requires environment variable 'ANTHROPIC_API_KEY'",
        '    to be set but it is not',
      ].join('\n'),
    )

    expect(new JobFailed(job).message).toBe(
      "BamlError: LLM client 'Sonnet' requires environment variable 'ANTHROPIC_API_KEY'",
    )
  })

  test('a line of quoted source that mentions an error does not win', () => {
    // Why the pattern is anchored: the source line contains the word `Error` in the middle
    // and the real fault names it at the start.
    const job = failed(
      [
        '    raise ValueError(f"{name}: model must be one of {known}")',
        'ValueError: plan: model must be one of gpt, opus, sonnet',
      ].join('\n'),
    )

    expect(new JobFailed(job).message).toBe(
      'ValueError: plan: model must be one of gpt, opus, sonnet',
    )
  })

  test('a job that said nothing at all still has a headline', () => {
    expect(new JobFailed(failed('   \n\n')).message).toBe(
      'the plan job ended failed without saying why',
    )
  })

  test('the whole log is carried, not only the line that was chosen', () => {
    // The screen offers the rest of it under the headline, so it has to survive.
    const job = failed('first\nValueError: second\nthird')

    expect(new JobFailed(job).log).toBe('first\nValueError: second\nthird')
  })
})

// --- what the engine says wins (S-65) -------------------------------------------------

/// A job the engine ended, carrying the sentence the engine wrote about the ending.
const ended = (status: Job['status'], note: string | null, log: string): Job => ({
  ...failed(log),
  kind: 'daily',
  status,
  note,
})

// A `daily` run's stderr where pattern 3 fell over, its traceback was printed, its verdicts
// were salvaged and the run carried on — which is what `daily.py` does, so this is the
// ordinary shape of a long run's log and not an unlucky one.
const SALVAGED = [
  'PROGRESS 1/9',
  'SPENT 0.041000',
  'Traceback (most recent call last):',
  'ValueError: pattern 3 asked about a column that is not there',
  'pattern 3 refund window: failed after reading 40 calls, whose verdicts have been applied',
  'PROGRESS 3/9',
  'SPENT 0.213000',
].join('\n')

const ABANDONED_NOTE =
  'the process running this job is gone; the $0.2130 it had reported spending by then has ' +
  'been booked, and anything it spent after that is lost'

describe('an ending the engine wrote', () => {
  test('is headlined by the engine, not by a traceback the brain recovered from', () => {
    // The defect. The analyst was told `ValueError: pattern 3 …` — a pattern that failed,
    // was salvaged and was left behind six lines ago — instead of being told that graphify
    // had restarted under the run and that $0.2130 had been booked against the org.
    const job = ended('abandoned', ABANDONED_NOTE, SALVAGED + '\n' + ABANDONED_NOTE)

    expect(new JobFailed(job).message).toBe(ABANDONED_NOTE)
  })

  test('wins even when its own line never reached the log', () => {
    // The row is written in the transaction that books the cost; the log copy is
    // best-effort. The headline comes off the row, so losing the copy costs nothing.
    const job = ended('abandoned', ABANDONED_NOTE, SALVAGED)

    expect(new JobFailed(job).message).toBe(ABANDONED_NOTE)
  })

  test('still carries the whole log underneath it', () => {
    const job = ended('abandoned', ABANDONED_NOTE, SALVAGED)

    expect(new JobFailed(job).log).toBe(SALVAGED)
  })
})

describe('an ending the brain wrote itself', () => {
  test('is still headlined by the brain, which is what the null is for', () => {
    // The guess is right here and has to stay. `note` is null exactly when the engine had
    // nothing to add, and then the brain's own complaint is the answer.
    const job = ended('failed', null, SALVAGED)

    expect(new JobFailed(job).message).toBe(
      'ValueError: pattern 3 asked about a column that is not there',
    )
  })

  test('with nothing in the log falls back to the status, not to a note that is not there', () => {
    expect(new JobFailed(ended('failed', null, '  \n\n')).message).toBe(
      'the daily job ended failed without saying why',
    )
  })
})

// --- a job somebody stopped (S-63) ----------------------------------------------------

describe('a job that was stopped while it was working', () => {
  afterEach(() => vi.unstubAllGlobals())

  /** `GET /api/jobs/{id}` answering with one status and nothing else. */
  const answering = (status: Job['status']) =>
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({ ...failed('nothing went wrong'), status }),
      })),
    )

  test('is not reported as a failure to a screen that was not waiting for it', async () => {
    // Somebody pressed a button. The screen that pressed it waits for `stopped` itself;
    // any other screen watching the same job is no longer owed an answer, and a red panel
    // saying "the plan job ended stopped without saying why" would be wrong twice over.
    answering('stopped')

    await expect(
      settle(1, ['done'], () => true, () => {}),
    ).rejects.toBeInstanceOf(Cancelled)
  })

  test('and the statuses that really are failures still are', async () => {
    answering('failed')

    await expect(
      settle(1, ['done'], () => true, () => {}),
    ).rejects.toBeInstanceOf(JobFailed)
  })

  test('a screen that is waiting for a stop is handed the job, not an exception', async () => {
    // The wizard's own watch names `stopped` among the endings it wants, because the button
    // that caused it is on that page and the row carries what the run cost.
    answering('stopped')

    const job = await settle(1, ['done', 'stopped'], () => true, () => {})
    expect(job.status).toBe('stopped')
  })
})
