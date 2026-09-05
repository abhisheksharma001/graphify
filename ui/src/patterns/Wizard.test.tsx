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

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import Wizard from './Wizard'
import type { Job, Plan } from '../api'

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

/** The engine, as far as this component can tell. Every start answers with a job id and
 * every job is already finished, so nothing here has to wait out a poll. */
function stubEngine(costs: (number | null)[] = [0.0043, 0.0051]) {
  sent = []
  let started = 0
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

      if (method === 'POST' && url.includes('/api/patterns/')) {
        started += 1
        return answer({ id: started, status: 'running' })
      }
      const job = /\/api\/jobs\/(\d+)$/.exec(url)
      if (job) {
        const id = Number(job[1])
        return answer(done(id, PLAN, costs[id - 1] ?? null))
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
