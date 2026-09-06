// The pattern wizard: one line of plain English in, and out the other end a rule the
// engine re-counts every day for nothing.
//
// Two steps. Step 1 says which calls and which model. Step 2 is the conversation — the
// criterion goes to the brain, a plan comes back as rows to argue with, the questions on
// the left redraw the table on the right — and none of that reads a single call.
//
// **The spending is two clicks on one button, and the price is on it before the click that
// costs.** The first click starts the labelling job, which the engine parks the instant the
// brain has printed what the run would cost: the child is alive with its stdin open, having
// read nothing and bought nothing. The button then carries that figure, and the second
// click is the go. It is sent exactly once — `POST /api/jobs/{id}/go` is the only thing in
// the system that lets a model read a call, and a double-click on a slow network must not
// be two of them.
//
// Quoting only on a deliberate click, rather than the moment the plan is good enough, is
// the whole reason for the second click. A parked job holds a live child for half an hour
// and there is no cancel; four of them is the engine's limit. Quoting on every clarify that
// happened to land above the gate would wedge the engine with jobs nobody asked for.
//
// **The price is a ceiling and is drawn as one.** The brain prices a batch's output at the
// `max_tokens` it cannot exceed, so a run of short calls quotes several times what it
// spends. The figure that comes back in `usd` afterwards is the real one, and both are
// shown — an estimate quietly replaced by a smaller truth teaches nobody anything.

import { useEffect, useRef, useState } from 'react'
import * as api from '../api'
import { MODELS, Unauthorized } from '../api'
import type { Assistant, Job, JobStatus, Labelled, Plan, Synthesized } from '../api'
import { DASH } from '../format'
import { Cancelled, JobFailed, settle } from '../jobs'
import PlanTable from './PlanTable'

/** The gate. Under this the wizard reads nothing, whatever is clicked. A plan the model is
 * unsure of labels the wrong calls, and those labels are both the part that costs money
 * and the yardstick the rule is then measured against — a wrong one twice over. */
export const GATE = 0.95

/** How many of the newest calls to label when the analyst has not pasted ids. Enough for
 * the agreement figure to mean something, few enough to be cheap. */
const DEFAULT_SAMPLE = '25'

/** Dollars. Required by the brain and with no default there — a cap a caller can leave out
 * is a cap that gets left out — so the wizard has to name one. It is checked before every
 * batch, and it covers each step on its own rather than the wizard as a whole. */
const DEFAULT_CAP = '2.00'

/** Four places, because these are fractions of a cent and rounding one to two would show
 * `$0.00` for a run that cost money — and a price nobody reported is a dash for the same
 * reason, which is the dashboard's rule and is no different here for being on a button.
 * `DASH` comes from `format.ts` so there is one em dash in this codebase and not two. */
const money = (usd: number | null) => (usd === null ? DASH : `$${usd.toFixed(4)}`)

/** The plan out of a finished `plan` or `clarify` job.
 *
 * The brain returns what the message cost sitting beside the plan's own fields, the same
 * shape `label` returns. It comes off here: the cost belongs to the job row, where the
 * engine has already booked it into the day's spend, and a plan carrying it would carry
 * it back into the next `clarify` as part of a plan the brain would have to ignore. */
function planOf(output: unknown): Plan {
  const { usd: _usd, ...rest } = output as Plan & { usd?: number }
  return rest as Plan
}

const percent = (x: number) => `${Math.round(x * 100)}%`

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** One attempt at buying labels, from the price to the pattern.
 *
 * `of` is the settings it belongs to. Everything else in it was true of those settings and
 * of no others, which is why they travel together rather than as four pieces of state that
 * can disagree. */
type Run = {
  of: string
  /** The labelling job, then the synthesising one: whichever is being watched. */
  job: Job
  /** The ids the quote was priced against. Kept because `GET /api/jobs/{id}` answers with
   * where a job got to, not with what it was asked to do. */
  ids: string[]
  labelled: Labelled | null
  saved: Synthesized | null
}

export default function Wizard({
  org,
  assistants,
  onError,
}: {
  org: number
  assistants: Assistant[]
  onError: (e: unknown) => void
}) {
  // Step 1: which calls, which model, how much.
  const [step, setStep] = useState<1 | 2>(1)
  const [assistantIds, setAssistantIds] = useState<string[]>([])
  const [source, setSource] = useState<'last' | 'ids'>('last')
  const [sample, setSample] = useState(DEFAULT_SAMPLE)
  const [pasted, setPasted] = useState('')
  const [readPrompt, setReadPrompt] = useState(false)
  const [model, setModel] = useState<string>(MODELS[0])
  const [cap, setCap] = useState(DEFAULT_CAP)

  // Step 2: the conversation.
  const [criterion, setCriterion] = useState('')
  const [plan, setPlan] = useState<Plan | null>(null)
  /** What this conversation has cost, and what its last message cost. Read off the job
   *  rows rather than worked out here: `cost_usd` is the figure the engine booked into
   *  the day's spend, and a second one in the browser that disagreed with it would be the
   *  one nobody could reconcile. Null until a message has been paid for — no messages yet
   *  is not a spend of zero. */
  const [chat, setChat] = useState<{ last: number; total: number } | null>(null)
  const [answers, setAnswers] = useState<Record<string, string>>({})

  // The spending, and what came of it. One object, because a quote, the calls it was
  // priced against, the labels it bought and the pattern written from them are one run and
  // are never separately true.
  const [run, setRun] = useState<Run | null>(null)
  const [name, setName] = useState('')

  const [busy, setBusy] = useState<string | null>(null)
  /** What went wrong, and the whole of what the brain wrote while it did. */
  const [failed, setFailed] = useState<{ message: string; log: string } | null>(null)

  // Polls check this before every step and again after every await, so closing the wizard
  // stops the loop rather than leaving it writing into a component nobody is looking at.
  const open = useRef(true)
  useEffect(() => {
    open.current = true
    return () => {
      open.current = false
    }
  }, [])

  /** The go, once. A ref and not a piece of state: it has to be true for the very next
   * event, and a re-render is too late for the second click of a double-click. */
  const went = useRef(false)

  // What a run is about. A quote is a price for a plan, a model and a set of calls; change
  // any of them and the figure on the button describes something that is no longer on
  // screen. Comparing this to the run's own tag is what "stale" means here, so there is no
  // effect racing the edit that caused it — the same trick the dashboard plays with its
  // query string.
  const settings = JSON.stringify([
    criterion.trim(),
    plan,
    model,
    source,
    sample.trim(),
    pasted,
    assistantIds,
    cap,
  ])

  // A stale run is not shown and not acted on. Its parked job is left alone: it has spent
  // nothing, and half an hour from now the engine marks it `expired` for exactly this case.
  const live = run !== null && run.of === settings ? run : null

  /** Update the job inside the live run — what the polls write, so the progress bar moves.
   * A tick that arrives after the run went stale is dropped rather than resurrecting it. */
  const tick = (job: Job) => setRun((r) => (r === null || r.of !== settings ? r : { ...r, job }))

  /** `settle`, with this wizard's liveness bound in, because every call wants the same one
   * and forgetting it is how a closed screen keeps polling. */
  const watch = (id: number, until: JobStatus[], onTick: (job: Job) => void = () => {}) =>
    settle(id, until, () => open.current, onTick)

  function fail(e: unknown) {
    if (e instanceof Cancelled) return
    if (e instanceof Unauthorized) onError(e)
    else setFailed({ message: message(e), log: e instanceof JobFailed ? e.log : '' })
  }

  /** Run one step, with the screen saying which one and refusing a second while it runs. */
  async function during(what: string, work: () => Promise<void>) {
    setFailed(null)
    setBusy(what)
    try {
      await work()
    } catch (e) {
      fail(e)
    } finally {
      if (open.current) setBusy(null)
    }
  }

  const pastedIds = () => [
    ...new Set(
      pasted
        .split(/[\s,]+/)
        .map((id) => id.trim())
        .filter(Boolean),
    ),
  ]

  /** How many calls the button offers to read. Before the quote this is what the settings
   * ask for; `last` may well find fewer, and once a quote exists the count is the ids that
   * were actually priced. */
  const count =
    live?.ids.length ?? (source === 'ids' ? pastedIds().length : Number(sample.trim()) || 0)

  /** The ids to label. Pasted ones as typed; otherwise the newest N of the selection, asked
   * of the engine so the wizard reads the same calls the dashboard would show. */
  async function callIds(): Promise<string[]> {
    if (source === 'ids') return pastedIds()
    const params = new URLSearchParams({
      org: String(org),
      last: sample.trim() || DEFAULT_SAMPLE,
    })
    for (const id of assistantIds) params.append('assistant_id', id)
    const rows = await api.calls(params)
    return rows.map((row) => row.id)
  }

  /** The selected assistants' system prompts, under their names, or nothing.
   *
   * Read one assistant at a time and only when the box is ticked: a prompt runs to tens of
   * kilobytes and no picker should be carrying them all. Several selected means several
   * prompts, each labelled, because the model is being asked what these agents were told
   * to do and one of them is not an answer for the others. */
  async function systemPrompt(): Promise<string | undefined> {
    if (!readPrompt || assistantIds.length === 0) return undefined
    const parts: string[] = []
    for (const id of assistantIds) {
      const { system_prompt } = await api.assistantPrompt(org, id)
      if (!system_prompt) continue
      const who = assistants.find((a) => a.id === id)?.name ?? id
      parts.push(`# ${who}\n\n${system_prompt}`)
    }
    return parts.length > 0 ? parts.join('\n\n') : undefined
  }

  /** Add what one message cost to the conversation's running total.
   *
   * A job that reported no cost is left out of both figures rather than counted as
   * nothing: an unreported spend is unknown, and the spec's rule about a missing value is
   * the same rule here as it is in a table cell. */
  const charge = (usd: number | null) =>
    setChat((prior) => (usd === null ? prior : { last: usd, total: (prior?.total ?? 0) + usd }))

  const draft = () =>
    during('Planning…', async () => {
      const line = criterion.trim()
      const prompt = await systemPrompt()
      // Left out rather than sent empty: the brain skips the prompt block entirely for an
      // absent one, and a heading with nothing under it is not the same absence.
      const asked = { criterion: line, model, max_usd: Number(cap) }
      const body = prompt ? { ...asked, system_prompt: prompt } : asked
      const { id } = await api.startPlan(org, body)
      const done = await watch(id, ['done'])
      setPlan(planOf(done.output))
      setAnswers({})
      charge(done.cost_usd)
    })

  const revise = () =>
    during('Revising the plan…', async () => {
      if (plan === null) return
      const given = plan.questions
        .map((question) => ({ question, answer: (answers[question] ?? '').trim() }))
        .filter((a) => a.answer !== '')
      if (given.length === 0) {
        throw new Error('Answer at least one of the questions, or reword the criterion.')
      }
      const { id } = await api.startClarify(org, {
        criterion: criterion.trim(),
        plan,
        answers: given,
        model,
        max_usd: Number(cap),
      })
      const done = await watch(id, ['done'])
      setPlan(planOf(done.output))
      setAnswers({})
      charge(done.cost_usd)
    })

  /** Click one: start the labelling job and let it park on its price. Reads nothing. */
  const priceIt = () =>
    during('Pricing the run…', async () => {
      if (plan === null) return
      const ids = await callIds()
      if (ids.length === 0) throw new Error('That selection has no calls in it.')
      const { id } = await api.startLabel(org, {
        criterion: criterion.trim(),
        plan,
        call_ids: ids,
        model,
        max_usd: Number(cap),
      })
      const parked = await watch(id, ['waiting'])
      went.current = false
      setRun({ of: settings, job: parked, ids, labelled: null, saved: null })
    })

  /** Click two: the go. Guarded, because this is the call that spends. */
  const spend = () =>
    during('Reading the calls…', async () => {
      if (live === null || went.current) return
      went.current = true
      await api.go(live.job.id)
      const done = await watch(live.job.id, ['done'], tick)
      const labelled = done.output as Labelled
      setRun((r) => (r === null || r.of !== settings ? r : { ...r, job: done, labelled }))
    })

  /** The other answer to click two. Turns the quote down: the engine kills the child with
   * its stdin still open, having read nothing, and the slot the job was holding is free
   * for the next quote rather than held for the half hour the engine would otherwise wait.
   */
  const decline = () =>
    during('Letting it go…', async () => {
      if (live === null) return
      // A refusal here means the job is already gone — it expired while this page sat
      // open, or the go beat this click to the map. Either way the answer to "stop it" is
      // that it is stopped, so the wizard goes back rather than reporting a failure to do
      // something already done.
      await api.stop(live.job.id).catch(() => {})
      setRun(null)
      went.current = false
    })

  /** Save. The brain writes the `patterns` row itself as the last thing `synthesize` does,
   * so this button is the one that creates the pattern — which is why it, too, shows what
   * it costs before it is pressed. */
  const save = () =>
    during('Writing the rule…', async () => {
      if (plan === null || live?.labelled == null) return
      const { id } = await api.startSynthesize(org, {
        criterion: criterion.trim(),
        plan,
        labels: live.labelled.labels,
        model,
        max_usd: Number(cap),
        org_id: org,
        name: name.trim(),
        assistant_ids: assistantIds,
      })
      const done = await watch(id, ['done'], tick)
      const saved = done.output as Synthesized
      setRun((r) => (r === null || r.of !== settings ? r : { ...r, job: done, saved }))
    })

  function restart() {
    setStep(1)
    setCriterion('')
    setPlan(null)
    setAnswers({})
    setRun(null)
    setName('')
    setFailed(null)
    went.current = false
  }

  const ready = plan !== null && plan.confidence >= GATE && plan.expressible
  const capOk = Number(cap) > 0 && Number.isFinite(Number(cap))
  /** The figure the go would be against. Null means the job parked without a price
   * reaching its log — the engine's invariant broken, not a run that costs nothing — and a
   * shown cost is a precondition of the go rather than decoration beside it. Kept apart
   * from the button so the same null decides both what is drawn and whether it is
   * clickable, which are one fact and were two. */
  const priced = live === null ? null : live.job.estimate_usd

  return (
    <section className="wizard">
      <div className="wizard-left">
        {step === 1 ? (
          <div className="card">
            <h3>1 &middot; Which calls</h3>

            <div className="field">
              <label htmlFor="w-assistants">Assistants</label>
              <select
                id="w-assistants"
                multiple
                value={assistantIds}
                onChange={(e) =>
                  setAssistantIds([...e.target.selectedOptions].map((o) => o.value))
                }
              >
                {assistants.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name ?? a.id}
                  </option>
                ))}
              </select>
              <span className="hint">
                {assistantIds.length === 0
                  ? 'all assistants in this org'
                  : `${assistantIds.length} selected`}
              </span>
            </div>

            <fieldset className="field">
              <legend className="label">Calls to read</legend>
              {/* Two answers to one question, so picking either clears the other. Typing in
                  a box picks it: reaching for the number and then wondering why the paste
                  box won is the kind of thing nobody reports and everybody hits. */}
              <div className="row">
                <label htmlFor="w-last">
                  <input
                    type="radio"
                    name="w-source"
                    checked={source === 'last'}
                    onChange={() => setSource('last')}
                  />{' '}
                  the newest
                </label>
                <input
                  id="w-last"
                  type="number"
                  min="1"
                  className="last-custom"
                  value={sample}
                  onChange={(e) => setSample(e.target.value)}
                  onFocus={() => setSource('last')}
                />
              </div>
              <label htmlFor="w-ids">
                <input
                  type="radio"
                  name="w-source"
                  checked={source === 'ids'}
                  onChange={() => setSource('ids')}
                />{' '}
                these call ids
              </label>
              <textarea
                id="w-ids"
                rows={3}
                value={pasted}
                onChange={(e) => setPasted(e.target.value)}
                onFocus={() => setSource('ids')}
                placeholder="One per line, or separated by commas"
              />
            </fieldset>

            <div className="field">
              <span className="label">The agent&rsquo;s prompt</span>
              <label htmlFor="w-prompt">
                <input
                  id="w-prompt"
                  type="checkbox"
                  checked={readPrompt}
                  onChange={(e) => setReadPrompt(e.target.checked)}
                  disabled={assistantIds.length === 0}
                />{' '}
                read it, so the plan knows what the agent was told to do
              </label>
              <span className="hint">
                {assistantIds.length === 0
                  ? 'Select an assistant above and this can be read.'
                  : 'Sent with the criterion. Read once, here, and never carried by the picker.'}
              </span>
            </div>

            <div className="field">
              <label htmlFor="w-model">Model</label>
              <select id="w-model" value={model} onChange={(e) => setModel(e.target.value)}>
                {MODELS.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </div>

            <div className="field">
              <label htmlFor="w-cap">Spend cap, per step</label>
              <input
                id="w-cap"
                type="number"
                min="0"
                step="0.5"
                value={cap}
                onChange={(e) => setCap(e.target.value)}
              />
              <span className="hint">
                Dollars, checked before every batch and never after. A step quoted above it
                is refused outright rather than started and stopped halfway.
              </span>
            </div>

            <button type="button" onClick={() => setStep(2)} disabled={!capOk}>
              Next
            </button>
          </div>
        ) : (
          <div className="card">
            <h3>2 &middot; What to count</h3>

            <div className="field">
              <label htmlFor="w-criterion">In a line</label>
              <textarea
                id="w-criterion"
                rows={3}
                value={criterion}
                onChange={(e) => setCriterion(e.target.value)}
                placeholder="Calls where the caller asked for a person and did not get one"
              />
            </div>

            <div className="row">
              <button type="button" onClick={() => setStep(1)}>
                Back
              </button>
              <button
                type="button"
                onClick={draft}
                disabled={busy !== null || criterion.trim() === ''}
              >
                {plan === null ? 'Draft the plan' : 'Start over from this line'}
              </button>
            </div>

            {/* The price, before the click that costs and after it. Drafting and revising
                read no transcripts, so they are a few cents rather than a few dollars —
                but a few cents a message with nothing said about it is how a wizard left
                open all afternoon becomes a line on a bill nobody can account for. The
                model is named because the picker that chose it is back on step one and
                the price here is its rate: Opus messages cost two and a half times what
                the same message costs on Sonnet. */}
            <p className="hint">
              {chat === null
                ? `Each message goes to ${model} and costs a few cents, refused above` +
                  ` ${money(Number(cap))}.`
                : `Last message ${money(chat.last)} · this conversation ${money(chat.total)},` +
                  ` each on ${model} and refused above ${money(Number(cap))}.`}
            </p>

            {plan !== null && plan.questions.length > 0 && (
              <div className="questions">
                <h4>What it would have to ask</h4>
                {/* Numbered ids, keyed by the question itself: a question is the only
                    stable name an answer has, and it is not a thing an `id` can hold. */}
                {plan.questions.map((question, i) => (
                  <div className="field" key={question}>
                    <label htmlFor={`w-q-${i}`}>{question}</label>
                    <input
                      id={`w-q-${i}`}
                      type="text"
                      value={answers[question] ?? ''}
                      onChange={(e) =>
                        setAnswers((a) => ({ ...a, [question]: e.target.value }))
                      }
                    />
                  </div>
                ))}
                <button type="button" onClick={revise} disabled={busy !== null}>
                  Answer and redraw
                </button>
              </div>
            )}

            {plan !== null && live?.labelled == null && (
              <div className="spend">
                {/* One button, two clicks, and the price is on it before the click that
                    costs. The first starts the job and lets it park on its own quote,
                    having read nothing; the second is the go. Disabled under the gate,
                    which is a fact about the plan and not about the network — and disabled
                    again when the quote came back without a figure, because the button that
                    cannot say what something costs must not be the button that buys it. */}
                <button
                  type="button"
                  className="go"
                  onClick={live === null ? priceIt : spend}
                  disabled={!ready || busy !== null || (live !== null && priced === null)}
                >
                  {live === null
                    ? `Read ${count} calls`
                    : `Read ${count} calls · up to ${money(priced)}`}
                </button>
                {/* The no. Present whenever there is a quote, including the one that came
                    back without a price — especially then, since that is the run there is
                    no way to approve. Not disabled by the gate: declining is the one
                    answer that is always available and always costs nothing. */}
                {live !== null && (
                  <button type="button" className="no" onClick={decline} disabled={busy !== null}>
                    Not now
                  </button>
                )}
                {live !== null && priced === null && (
                  <p className="hint">
                    This run parked without a price, so there is nothing to approve and
                    nothing has been read. Say “Not now” and price it again; the parked job
                    bought nothing, and letting it go frees the slot it is holding.
                  </p>
                )}
                {live !== null && priced !== null && (
                  <p className="hint">
                    A ceiling, not a forecast: output is priced at the most a batch could
                    return, so short calls usually cost a fraction of it. Nothing has been
                    read yet, and leaving this page reads nothing — though “Not now” hands
                    the slot back, where leaving holds it for half an hour.
                  </p>
                )}
                {live?.job.progress && busy !== null && (
                  <p className="hint">
                    <progress value={live.job.progress.done} max={live.job.progress.of} />{' '}
                    {live.job.progress.done} of {live.job.progress.of}
                  </p>
                )}
              </div>
            )}

            {live?.labelled != null && live.saved === null && (
              <div className="labelled">
                <h4>Read</h4>
                <p>
                  {live.labelled.labels.filter((l) => l.match).length} of{' '}
                  {live.labelled.labels.length} labelled calls match. It cost{' '}
                  {money(live.labelled.usd)}.
                </p>
                {live.labelled.no_transcript.length > 0 && (
                  <p className="hint">
                    {live.labelled.no_transcript.length} had no transcript to read.
                  </p>
                )}
                {live.labelled.stopped && (
                  <p className="hint">Stopped early: {live.labelled.stopped}.</p>
                )}

                <div className="field">
                  <label htmlFor="w-name">Name this pattern</label>
                  <input
                    id="w-name"
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Asked for a person"
                  />
                </div>
                <button
                  type="button"
                  className="go"
                  onClick={save}
                  disabled={busy !== null || name.trim() === ''}
                >
                  Save &middot; at most ${Number(cap).toFixed(2)}
                </button>
                <p className="hint">
                  Saving writes the rule out of these labels and measures it against them.
                  One model call on quotes already paid for, priced against the worst case
                  and refused if that comes to more than the cap.
                </p>
              </div>
            )}

            {live?.saved != null && (
              <div className="saved">
                <h4>Saved</h4>
                <p>
                  The rule agrees with {percent(live.saved.agreement)} of the sample &mdash;{' '}
                  {live.saved.agreed} of {live.saved.of}. It matched{' '}
                  {live.saved.matched_by_rule} calls where the model matched{' '}
                  {live.saved.matched_by_model}. It cost {money(live.saved.usd)}.
                </p>
                {live.saved.refined && (
                  <p className="hint">The first rule was refined: {live.saved.reason}</p>
                )}
                <p className="hint">
                  Chart: {live.saved.chart.kind} &mdash; {live.saved.chart.title}
                </p>
                <pre className="mono rule">{JSON.stringify(live.saved.rule, null, 2)}</pre>
                <button type="button" onClick={restart}>
                  Start another
                </button>
              </div>
            )}

            {busy !== null && <p className="hint">{busy}</p>}
            {failed !== null && (
              <div className="failed">
                <p className="error">{failed.message}</p>
                {failed.log !== '' && (
                  <details>
                    <summary>Everything the brain wrote</summary>
                    <pre className="mono rule">{failed.log}</pre>
                  </details>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="wizard-right">
        <PlanTable plan={plan} gate={GATE} />
      </div>
    </section>
  )
}
