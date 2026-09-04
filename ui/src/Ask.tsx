// The ask box: one question about the calls on screen, priced before it is asked.
//
// Everything else that spends in graphify quotes by starting the brain and parking it —
// the child is alive with its stdin open while somebody reads the figure, and the go goes
// down that pipe. This screen cannot work that way. A question is a thing people try,
// reword and abandon, and four abandoned quotes would be the engine's whole job budget
// held by questions nobody asked. So the price comes from the engine, on the request,
// with nothing started: **Cancel below sends nothing at all.**
//
// The quote is tagged with the filters, the question and the model it was taken for, and
// it is only shown while all three still match. A price is about a selection, and a
// figure left on screen after the window moved would be a figure for calls nobody is
// looking at any more.
//
// The answer rests on two different kinds of evidence and says so out loud, because they
// are not equally good. The statistics describe the whole selection. The transcripts are
// the *shortest* calls of it — that is how the most of them fit under one token cap — so
// they are a skewed sample, and both the prompt and the line under the box say so.

import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import * as api from './api'
import { MODELS, Unauthorized } from './api'
import type { AskQuote, Answered, Job } from './api'
import Answer from './Answer'
import { Cancelled, JobFailed, settle } from './jobs'

/** Four places: these are fractions of a cent, and two would show `$0.00` for a question
 * that cost money. */
const money = (usd: number) => `$${usd.toFixed(4)}`

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

const thousands = (n: number) => n.toLocaleString()

/** A quote, with what it was a quote *for*. The three parts are what make it stale: a new
 * window, a reworded question or a different model is a different price, and showing the
 * old one beside a button that spends would be quoting for calls nobody chose. */
type Priced = { of: string; quote: AskQuote }

/** One question that was actually asked, from the job to the answer.
 *
 * `agreed` is the figure that was on the button, carried here because that is the number
 * the answer has to be shown against. The brain quotes the same question again on its own
 * side and its figure is a little lower — the engine's is the ceiling — but "quoted at"
 * has to mean the price somebody looked at and clicked, not one they never saw. */
type Run = { of: string; agreed: number; job: Job; answered: Answered | null }

export default function Ask({
  query,
  bar,
  onError,
}: {
  /** The filter bar's query string. The question is about this selection and no other. */
  query: string
  bar: ReactNode
  onError: (e: unknown) => void
}) {
  const [question, setQuestion] = useState('')
  const [model, setModel] = useState<string>(MODELS[0])
  const [priced, setPriced] = useState<Priced | null>(null)
  const [run, setRun] = useState<Run | null>(null)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState<{ message: string; log: string } | null>(null)

  /** True while this screen is still the one on the page. A poll that outlives it is
   * abandoned rather than left to write into a component nobody is looking at. */
  const open = useRef(true)
  useEffect(() => {
    open.current = true
    return () => {
      open.current = false
    }
  }, [])

  const tag = `${query}|${model}|${question.trim()}`
  const quote = priced?.of === tag ? priced.quote : null
  const answer = run?.of === tag ? run : null
  const asked = question.trim().length > 0

  const params = useCallback(() => new URLSearchParams(query), [query])

  const fail = useCallback(
    (e: unknown) => {
      if (e instanceof Cancelled) return
      if (e instanceof Unauthorized) onError(e)
      else setFailed({ message: message(e), log: e instanceof JobFailed ? e.log : '' })
      setBusy(false)
    },
    [onError],
  )

  /** Step one: what would this cost. Starts nothing, so there is nothing to undo. */
  const price = () => {
    setFailed(null)
    setBusy(true)
    const of = tag
    api
      .askQuote(params(), { question: question.trim(), model })
      .then((got) => {
        if (!open.current) return
        setPriced({ of, quote: got })
        setRun(null)
        setBusy(false)
      })
      .catch(fail)
  }

  /** Step two: the click that spends. `max_usd` is the figure that was on the button. */
  const ask = async (agreed: number) => {
    setFailed(null)
    setBusy(true)
    const of = tag
    // The price is taken off the screen as the question goes: it has been agreed and
    // spent, and a live spend button sitting beside a finished answer is one somebody
    // clicks twice. Asking again means taking the price again, which is what "no history"
    // means from the paying end.
    setPriced(null)
    try {
      const started = await api.startAsk(params(), {
        question: question.trim(),
        model,
        max_usd: agreed,
      })
      const job = await settle(
        started.id,
        ['done'],
        () => open.current,
        (tick) => setRun({ of, agreed, job: tick, answered: null }),
      )
      setRun({ of, agreed, job, answered: job.output as Answered })
      setBusy(false)
    } catch (e) {
      fail(e)
    }
  }

  return (
    <>
      {bar}
      <div className="ask">
        <section className="card">
          <h2>Ask</h2>
          <p className="sub">
            One question about the calls this filter bar is showing. No history: each
            question is answered on its own, from the selection's statistics and a sample
            of its transcripts.
          </p>

          <div className="field">
            <label htmlFor="ask-q">Question</label>
            <textarea
              id="ask-q"
              rows={3}
              value={question}
              placeholder="Why are callers asking for a person?"
              onChange={(e) => setQuestion(e.target.value)}
            />
          </div>

          <div className="row">
            <div className="field">
              <label htmlFor="ask-model">Model</label>
              <select id="ask-model" value={model} onChange={(e) => setModel(e.target.value)}>
                {MODELS.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </div>
            <button type="button" onClick={price} disabled={!asked || busy || quote !== null}>
              What would this cost?
            </button>
          </div>

          {quote && (
            /* The price, and the two buttons. Cancel makes no request — the whole reason
               the quote above came from the engine rather than from a parked brain. */
            <div className="quote">
              <p>
                <b>{money(quote.usd)}</b> at most, on {quote.model}. It reads{' '}
                {quote.call_ids.length === 0
                  ? 'no transcripts'
                  : `${quote.call_ids.length} transcript${quote.call_ids.length === 1 ? '' : 's'}`}{' '}
                and {thousands(quote.tokens_in)} tokens of context.
              </p>
              <p className="sub">
                The transcripts are the shortest calls of this selection, so that the most
                of them fit. Every number in the answer comes from the statistics, which
                are about all of them.
                {quote.call_ids.length < quote.readable &&
                  ` The context filled up, so ${quote.readable - quote.call_ids.length} more of the sample did not fit.`}
              </p>
              <div className="row">
                <button type="button" className="go" onClick={() => ask(quote.usd)} disabled={busy}>
                  Ask · {money(quote.usd)}
                </button>
                <button type="button" onClick={() => setPriced(null)} disabled={busy}>
                  Cancel
                </button>
              </div>
            </div>
          )}

          {failed && (
            <div className="failed">
              <p className="error">{failed.message}</p>
              {failed.log && (
                <details>
                  <summary>What the brain said</summary>
                  <pre className="mono">{failed.log}</pre>
                </details>
              )}
            </div>
          )}
        </section>

        {answer && (
          <section className="card">
            <h2>Answer</h2>
            {answer.answered === null ? (
              <p className="notice">Reading…</p>
            ) : answer.answered.answer === null ? (
              <p className="notice">
                Nothing was asked: the question priced higher than what was approved. Take
                the price again.
              </p>
            ) : (
              <>
                <Answer markdown={answer.answered.answer} />
                <p className="sub">
                  {money(answer.answered.usd)} on {answer.answered.model}, over{' '}
                  {answer.answered.calls.length} transcript
                  {answer.answered.calls.length === 1 ? '' : 's'}
                  {` · quoted at ${money(answer.agreed)}`}.
                </p>
              </>
            )}
          </section>
        )}
      </div>
    </>
  )
}
