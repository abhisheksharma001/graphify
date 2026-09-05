// The headline on a failed job.
//
// `JobFailed`'s message is picked out of whatever the brain wrote to stderr by a regex
// its own comment calls "a guess, and a load-bearing one". That is exactly the kind of
// thing worth holding: the guess is right for the two shapes the brain actually produces
// and there is no type that says so.

import { describe, expect, test } from 'vitest'

import { JobFailed } from './jobs'
import type { Job } from './api'

const failed = (log: string): Job => ({
  id: 1,
  kind: 'plan',
  status: 'failed',
  progress: null,
  estimate_usd: null,
  cost_usd: null,
  output: null,
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
