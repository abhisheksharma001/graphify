// What the banner does with what the engine gives it.
//
// The banner exists because two engine failures — a boot sweep that could not clear the
// jobs a dead process left live, and a close that could not be written — used to reach
// stderr and nowhere else. Everything worth asserting here is about the case where
// something has gone wrong and the case where nothing has, because a banner that shows on
// an ordinary afternoon is one nobody reads on the day it matters.
//
// `fetch` is stubbed and nothing above it, for the same reason `Wizard.test.tsx` gives:
// stubbing `api.ts` would prove the component calls `notices()`, which was never the
// question. What is asserted is what a person ends up looking at.

import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'

import Notices from './Notices'
import type { Notices as Board } from './api'

const board = (notices: Board['notices'], dropped = 0): Board => ({ notices, dropped })

const SWEEP = {
  at: '2026-09-06T09:41:07.412Z',
  text: 'could not clear the jobs left behind by the last run: the queue is not writable. They still count against the limit of 4, so new jobs may be refused until this database is writable and graphify is started again.',
}

const CLOSE = {
  at: '2026-09-06T09:44:19.006Z',
  text: 'job 7: could not close this job out: the row is not writable. It stays `running` and holds one of the 4 job slots until graphify is started again.',
}

function stub(answer: Board | Error) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      if (answer instanceof Error) throw answer
      return { ok: true, status: 200, json: async () => answer } as Response
    }),
  )
}

beforeEach(() => stub(board([])))
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

test('an engine with nothing to report puts no banner on the page', async () => {
  const { container } = render(<Notices />)
  await waitFor(() => expect(fetch).toHaveBeenCalled())
  expect(container.querySelector('.notices')).toBeNull()
})

test('both failures reach the screen in the engine s own words', async () => {
  stub(board([CLOSE, SWEEP]))
  render(<Notices />)

  // The consequence, not only the error: this is the half of each sentence that tells the
  // operator a slot is gone until a restart, and it is the half that used to go to stderr.
  expect(await screen.findByText(/new jobs may be refused/)).toBeTruthy()
  expect(screen.getByText(/holds one of the 4 job slots/)).toBeTruthy()
  expect(screen.getByText(/could not write to its database/)).toBeTruthy()
})

test('notices the engine could not keep are counted on the page and not dropped twice', async () => {
  stub(board([SWEEP], 3))
  render(<Notices />)

  expect(await screen.findByText(/3 older notices were not kept/)).toBeTruthy()
})

test('a board that says nothing was dropped says nothing about dropping', async () => {
  stub(board([SWEEP]))
  render(<Notices />)

  await screen.findByText(/new jobs may be refused/)
  expect(screen.queryByText(/were not kept/)).toBeNull()
})

test('a notices request that fails leaves the page as it was', async () => {
  stub(new Error('the engine is not answering'))
  const { container } = render(<Notices />)

  await waitFor(() => expect(fetch).toHaveBeenCalled())
  // No banner and no thrown render. The route being unreachable is not itself news, and
  // the error line above belongs to requests a person actually made.
  expect(container.querySelector('.notices')).toBeNull()
})
