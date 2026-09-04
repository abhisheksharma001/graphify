// The plan, beside the conversation that produced it.
//
// Not a summary of the criterion — a summary is agreeable and tells you nothing. Every row
// here is one condition and what it does to the count, which is the only shape an analyst
// can actually disagree with, and disagreeing is the whole point of the step.
//
// The two verdicts under the table are the two the wizard spends against. A confidence
// below the gate means the model is not sure it understood; `expressible` false means it
// understood and the rule engine cannot check what it understood — which is fatal at any
// confidence, because the rule is what counts the calls for nothing afterwards.

import type { Plan } from '../api'

/** 0–1 as whole percent. Confidence is always a number in a plan, so there is nothing
 * missing here to render — and nothing missing to render as a zero. */
const percent = (x: number) => `${Math.round(x * 100)}%`

export default function PlanTable({ plan, gate }: { plan: Plan | null; gate: number }) {
  if (plan === null) {
    return (
      <div className="card plan">
        <h3>The plan</h3>
        <p className="hint">
          Say what you want counted, in a line. The plan appears here as rows you can argue
          with. Nothing is read and nothing is spent until a button says what it costs.
        </p>
      </div>
    )
  }

  const sure = plan.confidence >= gate

  return (
    <div className="card plan">
      <h3>The plan</h3>
      <table>
        <thead>
          <tr>
            <th scope="col">If</th>
            <th scope="col">Then</th>
          </tr>
        </thead>
        <tbody>
          {/* Keyed by position: a plan has no ids, and every redraw replaces the whole
              table rather than editing rows within it. */}
          {plan.rows.map((row, i) => (
            <tr key={i}>
              <td>{row.if_}</td>
              <td>{row.then}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {plan.reason && <p className="hint">{plan.reason}</p>}

      <dl className="verdicts">
        <div>
          <dt>Confidence</dt>
          <dd className={sure ? undefined : 'short'}>{percent(plan.confidence)}</dd>
        </div>
        <div>
          <dt>A rule can check it</dt>
          <dd className={plan.expressible ? undefined : 'short'}>
            {plan.expressible ? 'yes' : 'no'}
          </dd>
        </div>
      </dl>

      {!sure && (
        <p className="hint">
          Under {percent(gate)} nothing is read. Answer what it asked and the plan is
          redrawn.
        </p>
      )}
      {!plan.expressible && (
        <p className="hint">
          A row here is not something the rule engine can check, so the free daily count
          would not survive this wizard. Reword the criterion around what a transcript and
          the call record actually say.
        </p>
      )}
    </div>
  )
}
