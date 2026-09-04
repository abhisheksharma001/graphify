// Watching a brain job, and saying what went wrong when one ends somewhere its caller was
// not waiting for.
//
// Two screens do this now. The wizard buys labels and then a rule; the ask box buys one
// answer. Both need the same three things — a poll, a way to give up when the screen has
// moved on, and a headline for a failure — and one copy of them is what keeps the two from
// coming to disagree about what `waiting` means.

import * as api from './api'
import type { Job, JobStatus } from './api'

/** How often a running job is asked where it got to. Jobs move on the timescale of a model
 * call; under a second is noise on the wire and no news on the screen. */
const POLL_MS = 700

/** Thrown when the screen that started the watch has gone. Not a failure and not shown:
 * nobody is waiting for the answer any more. */
export class Cancelled extends Error {}

/** A line that opens with an exception's name. Anchored, so a line of quoted source that
 * happens to mention an error does not win over the one that names the fault. */
const NAMES_A_FAULT = /^[\w.]*(Error|Exception)\b/

/** What went wrong, in the brain's own words.
 *
 * A failed job's complaint is what it wrote to stderr, and the engine has already replaced
 * every key it handed the child with `***` on the way into that column. So showing the log
 * is safe, and the whole of it is offered under this headline.
 *
 * The headline is the last line naming an exception, and the last non-empty line only when
 * there is none. A guess, and a load-bearing one. The brain's own refusals are a single
 * tidy line and have no exception name, so they fall to the second branch and are quoted
 * whole. Anything it did not expect arrives as a Python traceback whose last line is the
 * tail of a wrapped sentence: "to be set but it is not" is a true last line and tells
 * nobody that no model key is configured, while "BamlError: LLM client 'Sonnet' requires
 * environment variable 'ANTHROPIC_API_KEY'" is four lines above it and is the answer.
 */
function complaint(job: Job): string {
  const lines = job.log
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
  const named = lines.filter((line) => NAMES_A_FAULT.test(line)).pop()
  return named ?? lines.pop() ?? `the ${job.kind} job ended ${job.status} without saying why`
}

/** A job that ended somewhere its caller was not waiting for, carried with its log so the
 * screen can offer the rest of what it said. */
export class JobFailed extends Error {
  readonly log: string
  constructor(job: Job) {
    super(complaint(job))
    this.log = job.log
  }
}

/** Watch one job until it reaches a state this caller was waiting for.
 *
 * `until` is the caller's business and not the job's: a labelling job is watched to
 * `waiting` first — parked on its price, nothing read — and then, after the go, to `done`.
 *
 * `running` and `waiting` both mean keep asking. The second matters because of an ordering
 * in the engine: `POST /api/jobs/{id}/go` answers as soon as the word is on its way to the
 * child, and the row goes back to `running` in the thread that was parked on it, after
 * that. The window is small — six goes in a row here were all `running` by the next
 * request — but it is real, and a client that read `waiting` as an ending would fail a
 * labelling run that was about to succeed. Anything else that is not `until` is an error,
 * carrying the brain's own last line.
 */
export async function settle(
  id: number,
  until: JobStatus[],
  alive: () => boolean,
  tick: (job: Job) => void,
): Promise<Job> {
  for (;;) {
    if (!alive()) throw new Cancelled()
    const job = await api.job(id)
    if (!alive()) throw new Cancelled()
    tick(job)
    if (until.includes(job.status)) return job
    if (job.status !== 'running' && job.status !== 'waiting') throw new JobFailed(job)
    await new Promise((resume) => setTimeout(resume, POLL_MS))
  }
}
