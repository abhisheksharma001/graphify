// What the wizard sends when it spends, and what it says the spending will cost.
//
// This is the surface S-33 and S-34 built and neither could verify. S-33 put a price on
// the chat; S-34 made that price the picked model's rate. Both were checked by one person
// driving one browser once, and the failure S-34 was fixing — a field the wizard never
// put on the request body — is precisely the failure a build and a linter cannot see.
//
// `fetch` is stubbed and nothing above it. Replacing `api.ts` would test that the wizard
// calls `startPlan`, which was already true on the day the bug shipped; what has to be
// asserted is the JSON that leaves the browser.
//
// S-36 added the second click. `POST /api/jobs/{id}/go` is the only call in the system
// that lets a model read a call, so the tests at the bottom are about when it is sent,
// when it is not, and how many times.

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import Wizard from './Wizard'
import type { Job, Labelled, Plan } from '../api'

/** One request the browser made, as the engine would have received it. */
type Sent = { method: string; url: string; body: Record<string, unknown> }

const PLAN: Plan & { usd: number } = {
  rows: [{ if_: 'the caller asked for a person', then: 'counts' }],
  questions: ['Does a voicemail count?'],
  confidence: 0.9,
  expressible: true,
  reason: 'One reading is unclear.',
  // The brain returns what the message cost beside the plan's own fields. `planOf` takes
  // it off, and a `clarify` that carried it back would be sending the brain a field it
  // would refuse.
  usd: 0.0043,
}

/** A plan the wizard will let you spend on. `PLAN` sits under `GATE` on purpose, which is
 * fine for the tests above — they never reach the spend button — and is no good for the
 * ones below, where a disabled button would pass for the wrong reason. */
const SURE: Plan & { usd: number } = { ...PLAN, confidence: 0.97 }

const LABELLED: Labelled = {
  labels: [
    { call_id: 'c1', match: true },
    { call_id: 'c2', match: false },
    { call_id: 'c3', match: true },
  ],
  no_transcript: [],
  no_label: [],
  not_reached: [],
  usd: 0.0312,
  batches: 1,
  model: 'sonnet',
  stopped: null,
}

const done = (id: number, output: unknown, cost: number | null): Job => ({
  id,
  kind: 'plan',
  status: 'done',
  progress: null,
  estimate_usd: 0.0438,
  cost_usd: cost,
  output,
  log: '',
  created_at: '2026-01-01T00:00:00Z',
  finished_at: '2026-01-01T00:00:02Z',
})

let sent: Sent[]

type StubOptions = {
  /** What `plan` and `clarify` answer with. */
  plan?: Plan & { usd: number }
  /** What a labelling job has parked on. `null` is the engine's invariant broken: a job
   * waiting for a go with no price for anyone to approve. */
  estimate?: number | null
  /** How many calls `GET /api/calls` finds for the selection. */
  found?: number
}

/** The engine, as far as this component can tell. Every start answers with a job id and
 * every job is already finished, so nothing here has to wait out a poll. */
function stubEngine(
  costs: (number | null)[] = [0.0043, 0.0051],
  { plan = PLAN, estimate = 0.1094, found = 3 }: StubOptions = {},
) {
  sent = []
  let started = 0
  /** What each started job was asked to be, by the id it was given. A labelling job parks
   * and a planning one does not, and `GET /api/jobs/{id}` is the same URL for both. */
  const kinds = new Map<number, string>()
  /** The labelling jobs that have been told to go. Before the go a labelling job answers
   * `waiting`, which is the state the whole two-click rule is about. */
  const gone = new Set<number>()

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string, init?: RequestInit) => {
      const url = String(input)
      const method = init?.method ?? 'GET'
      const body = init?.body ? JSON.parse(String(init.body)) : {}
      sent.push({ method, url, body })

      const answer = (data: unknown) => ({
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => data,
      })

      const start = /\/api\/patterns\/(\w+)(?:\?|$)/.exec(url)
      if (method === 'POST' && start) {
        started += 1
        kinds.set(started, start[1])
        return answer({ id: started, status: 'running' })
      }
      const go = /\/api\/jobs\/(\d+)\/go$/.exec(url)
      if (method === 'POST' && go) {
        gone.add(Number(go[1]))
        return answer({ id: Number(go[1]), status: 'running' })
      }
      const stop = /\/api\/jobs\/(\d+)\/stop$/.exec(url)
      if (method === 'POST' && stop) {
        return answer({ id: Number(stop[1]), status: 'expired' })
      }
      if (url.startsWith('/api/calls?')) {
        return answer(Array.from({ length: found }, (_, i) => ({ id: `c${i + 1}` })))
      }
      const job = /\/api\/jobs\/(\d+)$/.exec(url)
      if (job) {
        const id = Number(job[1])
        if (kinds.get(id) !== 'label') return answer(done(id, plan, costs[id - 1] ?? null))
        return answer(
          gone.has(id)
            ? { ...done(id, LABELLED, LABELLED.usd), kind: 'label' }
            : { ...done(id, null, null), kind: 'label', status: 'waiting', estimate_usd: estimate },
        )
      }
      throw new Error(`the wizard asked for something this test does not serve: ${url}`)
    }),
  )
}

/** The request body of the one `POST` to a brain function, by name. */
const posted = (fn: string) => {
  const match = sent.filter((r) => r.method === 'POST' && r.url.includes(`/api/patterns/${fn}`))
  expect(match, `expected exactly one POST to ${fn}`).toHaveLength(1)
  return match[0]
}

/** Every `POST .../go` the browser has made. The only call in the system that lets a model
 * read a call, so counting them is most of this file's second half. */
const goes = () => sent.filter((r) => r.method === 'POST' && r.url.endsWith('/go'))

/** Every `POST .../stop` the browser has made. Spends nothing; the point of counting them
 * is that the slot goes back rather than being held for the half hour the engine waits. */
const stops = () => sent.filter((r) => r.method === 'POST' && r.url.endsWith('/stop'))

/** The no beside the go. */
const noButton = () => screen.getByRole('button', { name: 'Not now' })

/** All the POSTs to one brain function, however many — `posted` above wants exactly one. */
const posts = (fn: string) =>
  sent.filter((r) => r.method === 'POST' && r.url.includes(`/api/patterns/${fn}`))

/** The one button that spends, whichever of its two clicks it is currently offering. */
const spendButton = () => screen.getByRole('button', { name: /^Read \d+ calls/ })

/** Step 1 with the model and cap chosen, then step 2 with the criterion typed and sent. */
async function draft({ model = 'sonnet', cap = '2.00', line = 'asked for a person' } = {}) {
  const user = userEvent.setup()
  render(<Wizard org={1} assistants={[]} onError={() => {}} />)

  await user.selectOptions(screen.getByLabelText('Model'), model)
  await user.clear(screen.getByLabelText('Spend cap, per step'))
  await user.type(screen.getByLabelText('Spend cap, per step'), cap)
  await user.click(screen.getByRole('button', { name: 'Next' }))

  await user.type(screen.getByLabelText('In a line'), line)
  await user.click(screen.getByRole('button', { name: 'Draft the plan' }))
  await screen.findByText(PLAN.rows[0].if_)
  return user
}

beforeEach(() => stubEngine())
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('the model and the cap picked in step 1', () => {
  test.each(['sonnet', 'opus', 'gpt'])('reach the plan request as %s', async (model) => {
    await draft({ model })

    // The whole of S-34, asserted where it can be seen: not that `startPlan` was called,
    // but that `model` is in the JSON the engine will hand the brain.
    expect(posted('plan').body).toMatchObject({ model, max_usd: 2, criterion: 'asked for a person' })
  })

  test('reach the clarify request too, on the second message of the conversation', async () => {
    const user = await draft({ model: 'opus', cap: '0.5' })

    await user.type(screen.getByLabelText(PLAN.questions[0]), 'no')
    await user.click(screen.getByRole('button', { name: 'Answer and redraw' }))
    await waitFor(() => expect(posted('clarify')).toBeDefined())

    expect(posted('clarify').body).toMatchObject({
      model: 'opus',
      max_usd: 0.5,
      criterion: 'asked for a person',
      answers: [{ question: PLAN.questions[0], answer: 'no' }],
    })
  })

  test('a cap typed in dollars goes as a number, not as the string it was typed in', async () => {
    // `max_usd` is validated by the brain as a positive number; `"0.25"` is a string and
    // would be refused with the model untouched, after a round trip nobody asked for.
    await draft({ cap: '0.25' })

    expect(posted('plan').body.max_usd).toBe(0.25)
  })

  test('the plan goes back to clarify without the cost that came with it', async () => {
    const user = await draft()

    await user.type(screen.getByLabelText(PLAN.questions[0]), 'no')
    await user.click(screen.getByRole('button', { name: 'Answer and redraw' }))
    await waitFor(() => expect(posted('clarify')).toBeDefined())

    expect(posted('clarify').body.plan).not.toHaveProperty('usd')
  })
})

describe('the price line', () => {
  test('names the model that was picked, before anything has been spent', async () => {
    const user = userEvent.setup()
    render(<Wizard org={1} assistants={[]} onError={() => {}} />)

    await user.selectOptions(screen.getByLabelText('Model'), 'opus')
    await user.click(screen.getByRole('button', { name: 'Next' }))

    // The picker is on step 1 and this line is on step 2, so the name is the only thing
    // telling a reader which model the few cents belong to.
    expect(screen.getByText(/Each message goes to opus/)).toBeTruthy()
  })

  test('says what the last message cost and what the conversation has cost', async () => {
    const user = await draft({ model: 'gpt' })

    expect(screen.getByText(/Last message \$0\.0043 · this conversation \$0\.0043/)).toBeTruthy()

    await user.type(screen.getByLabelText(PLAN.questions[0]), 'no')
    await user.click(screen.getByRole('button', { name: 'Answer and redraw' }))

    await screen.findByText(/this conversation \$0\.0094/)
    expect(screen.getByText(/each on gpt and refused above \$2\.0000/)).toBeTruthy()
  })

  test('a message whose cost nobody reported is left out rather than counted as nothing', async () => {
    // The same rule as `format.ts`: an unreported spend is unknown, and adding zero to the
    // total would be the dashboard's "render a missing value as 0" in another costume.
    stubEngine([null])
    await draft()

    expect(screen.getByText(/Each message goes to sonnet/)).toBeTruthy()
    expect(screen.queryByText(/Last message/)).toBeNull()
  })
})

describe('the two clicks that spend', () => {
  /** Through step 2 and the first of the two clicks: priced, parked, nothing read. */
  async function quoted(options: Parameters<typeof draft>[0] = {}) {
    const user = await draft(options)
    await user.click(spendButton())
    await screen.findByRole('button', { name: /up to/ })
    return user
  }

  beforeEach(() => stubEngine(undefined, { plan: SURE }))

  test('the first click prices the run and sends no go', async () => {
    await quoted()

    expect(posts('label')).toHaveLength(1)
    expect(posts('label')[0].body).toMatchObject({
      model: 'sonnet',
      max_usd: 2,
      call_ids: ['c1', 'c2', 'c3'],
    })
    // The job is parked with its stdin open, having read nothing and bought nothing. The
    // click that costs has not happened.
    expect(goes()).toHaveLength(0)
  })

  test('the price is on the button before the click that costs and not before the one that is free', async () => {
    const user = await draft()

    // Nothing has been quoted, so there is no figure to put on it — and the count is what
    // the settings ask for rather than what the selection turned out to hold.
    expect(spendButton().textContent).toBe('Read 25 calls')

    await user.click(spendButton())
    await screen.findByRole('button', { name: /up to/ })

    expect(spendButton().textContent).toBe('Read 3 calls · up to $0.1094')
    expect((spendButton() as HTMLButtonElement).disabled).toBe(false)
  })

  test('a double-click sends exactly one go', async () => {
    await quoted()

    const button = spendButton()
    // Both clicks inside one `act`, rather than two awaited `userEvent` clicks. React
    // flushes a discrete event synchronously, so two separate clicks would find the button
    // already `disabled` and the test would be asserting that attribute — which is a real
    // guard but not this one. Batched into a single act, no re-render happens between them
    // and the second click lands on a button that still looks clickable, which is the
    // double-click on a slow network the `went` ref was put there for.
    await act(async () => {
      fireEvent.click(button)
      fireEvent.click(button)
    })
    await screen.findByText(/2 of 3 labelled calls match/)

    expect(goes()).toHaveLength(1)
    expect(goes()[0].url).toBe('/api/jobs/2/go')
  })

  test('a setting changed after the quote takes the price off the button and the next click prices again', async () => {
    const user = await quoted()

    await user.type(screen.getByLabelText('In a line'), ' who asked twice')

    // The figure on the button was for a criterion that is no longer on screen. A button
    // that still said $0.11 would be quoting one run and buying another.
    expect(spendButton().textContent).toBe('Read 25 calls')

    await user.click(spendButton())
    await screen.findByRole('button', { name: /up to/ })

    expect(posts('label')).toHaveLength(2)
    // The first parked job is left alone rather than sent a go it was not priced for. It
    // bought nothing, and the engine expires it within the half hour.
    expect(goes()).toHaveLength(0)
  })

  test('a run that parked without a price shows a dash and offers no go', async () => {
    // Reachable rather than theoretical: the engine appends the brain's ESTIMATE line to
    // the job log and parks whether or not that write succeeded, and `estimate_usd` is read
    // back out of that log. Before S-36 this button read `up to $0.00` and the click behind
    // it was the go.
    stubEngine(undefined, { plan: SURE, estimate: null })
    await quoted()

    expect(spendButton().textContent).toBe('Read 3 calls · up to —')
    expect((spendButton() as HTMLButtonElement).disabled).toBe(true)
    expect(goes()).toHaveLength(0)

    // And it says so, rather than leaving a dead button to be read as a slow network.
    expect(screen.getByText(/parked without a price/)).toBeTruthy()
    expect(screen.queryByText(/A ceiling, not a forecast/)).toBeNull()
  })

  test('the no is offered once there is a quote, and not before', async () => {
    const user = await draft()

    // Nothing is parked yet, so there is nothing to decline: the only button here is the
    // one that would price the run.
    expect(screen.queryByRole('button', { name: 'Not now' })).toBeNull()

    await user.click(spendButton())
    await screen.findByRole('button', { name: /up to/ })
    expect((noButton() as HTMLButtonElement).disabled).toBe(false)
  })

  test('turning a quote down stops the job and leaves the wizard able to price again', async () => {
    const user = await quoted()

    await user.click(noButton())
    await screen.findByRole('button', { name: 'Read 25 calls' })

    // The job the analyst walked away from is told so, rather than left parked on a slot
    // for half an hour. Before S-38 this click did not exist and closing the tab was it.
    expect(stops()).toHaveLength(1)
    expect(stops()[0].url).toBe('/api/jobs/2/stop')
    // And the no is not a quiet yes.
    expect(goes()).toHaveLength(0)

    // Back where it was before the quote: no price on the button, and no button to decline.
    expect(screen.queryByRole('button', { name: 'Not now' })).toBeNull()

    // A second quote starts a second job, which is the point of giving the slot back.
    await user.click(spendButton())
    await screen.findByRole('button', { name: /up to/ })
    expect(posts('label')).toHaveLength(2)
    expect(goes()).toHaveLength(0)
  })

  test('a run that parked without a price can still be turned down', async () => {
    // The run there is no way to approve is the one most worth being able to let go of:
    // its go is disabled by S-36, so without this button the slot is held to the timer.
    stubEngine(undefined, { plan: SURE, estimate: null })
    const user = await quoted()
    expect((spendButton() as HTMLButtonElement).disabled).toBe(true)
    expect((noButton() as HTMLButtonElement).disabled).toBe(false)

    await user.click(noButton())
    await screen.findByRole('button', { name: 'Read 25 calls' })
    expect(stops()).toHaveLength(1)
    expect(goes()).toHaveLength(0)
  })
})