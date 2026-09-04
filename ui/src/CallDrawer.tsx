// One call, in full.
//
// The table can only carry what fits in a row. Everything else about a call is here: what
// was said, which tools ran and which of them failed, what Vapi made of it afterwards.
//
// A `<dialog>` rather than a styled `<div>`, because `showModal()` is what already
// implements the parts of a modal that are easy to get wrong — Escape closes it, focus
// stays inside it, and the page behind it goes inert — and none of that is worth
// reimplementing.
//
// D-3: the recording is a link out. The audio is never downloaded, never stored, and
// never played here.

import { useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import * as api from './api'
import { Unauthorized } from './api'
import { DASH, count, full, millis, money, seconds } from './format'
import { colour, display } from './groups'

/** Who Vapi puts in front of a line. Anything else at the head of a line is not a
 * speaker, and the line is treated as the previous turn continuing — so a colon inside a
 * sentence never invents a new speaker, and nothing in the transcript is ever dropped. */
const SPEAKERS = new Set(['ai', 'user', 'bot', 'assistant', 'system', 'human'])

type Turn = { speaker: string | null; text: string }

/** The transcript as turns. Vapi sends it as one string of `Speaker: line` rows; this is
 * the only place that shape is known about. */
function turns(transcript: string): Turn[] {
  const out: Turn[] = []
  for (const line of transcript.split('\n')) {
    const at = line.indexOf(':')
    const who = at > 0 ? line.slice(0, at).trim() : ''
    if (SPEAKERS.has(who.toLowerCase())) {
      out.push({ speaker: who, text: line.slice(at + 1).trim() })
      continue
    }
    const rest = line.trim()
    if (!rest) continue
    const last = out[out.length - 1]
    if (last) last.text += `\n${rest}`
    else out.push({ speaker: null, text: rest })
  }
  return out
}

/** `t+12.3s`, so a tool call can be found in the transcript by when it happened. */
const at = (v: number | null) => (v === null ? DASH : `t+${v.toFixed(1)}s`)

export default function CallDrawer({
  id,
  onClose,
  onError,
}: {
  id: string
  onClose: () => void
  onError: (e: unknown) => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [call, setCall] = useState<api.CallDetail | null>(null)
  /** A detail that would not load is this drawer's problem, not the page's: the charts
   * behind it are still true, so the message stays in here. A 401 is the exception — it
   * is the whole session's problem, and goes up to be signed out on. */
  const [error, setError] = useState<string | null>(null)

  // `showModal` throws on a dialog that is already open, and an effect runs twice under
  // StrictMode, so the open state is checked rather than assumed.
  useEffect(() => {
    const d = dialog.current
    if (d && !d.open) d.showModal()
  }, [])

  // Keyed by id upstream, so a different call is a different drawer: there is no
  // previous call's transcript to clear, because there is no previous instance.
  useEffect(() => {
    let live = true
    api
      .call(id)
      .then((c) => live && setCall(c))
      .catch((e: unknown) => {
        if (!live) return
        if (e instanceof Unauthorized) onError(e)
        else setError(e instanceof Error ? e.message : String(e))
      })
    return () => {
      live = false
    }
  }, [id, onError])

  return (
    <dialog
      ref={dialog}
      className="drawer"
      onClose={onClose}
      // The backdrop belongs to the dialog element, so a backdrop click reports the
      // dialog as its target. Comparing the point to the panel's box rather than
      // comparing targets is what keeps a click on the panel's own padding — which
      // reports the same target — from closing it.
      onClick={(e) => {
        const d = dialog.current
        if (!d) return
        const box = d.getBoundingClientRect()
        const inside =
          box.left <= e.clientX &&
          e.clientX <= box.right &&
          box.top <= e.clientY &&
          e.clientY <= box.bottom
        if (!inside) d.close()
      }}
    >
      <header>
        <div>
          <h2>Call</h2>
          <p className="hint mono">{id}</p>
        </div>
        <button className="close" onClick={() => dialog.current?.close()}>
          Close
        </button>
      </header>

      {error && <p className="error">{error}</p>}
      {!call && !error && <p className="notice">Loading…</p>}
      {call && <Body call={call} />}
    </dialog>
  )
}

function Body({ call }: { call: api.CallDetail }) {
  return (
    <>
      <dl className="facts">
        <Fact k="Started" v={call.created_at ? full.format(new Date(call.created_at)) : DASH} />
        <Fact k="Duration" v={seconds(call.duration_s)} />
        <Fact k="Assistant" v={call.assistant_name ?? call.assistant_id ?? DASH} />
        <Fact k="Status" v={call.status ?? DASH} />
        <Fact k="Type" v={call.call_type ?? DASH} />
        <Fact k="Turns" v={count(call.turns)} />
        <Fact k="Turn latency" v={millis(call.lat_turn_avg_ms)} />
        <Fact k="p50 / p95" v={`${millis(call.lat_turn_p50_ms)} / ${millis(call.lat_turn_p95_ms)}`} />
        <Fact k="Cost" v={money(call.cost)} />
        <Fact
          k="Ended"
          v={
            <>
              <i style={{ background: colour(display(call.ended_group ?? 'unknown')) }} />
              {call.ended_reason ?? call.ended_group ?? DASH}
            </>
          }
        />
        {call.transferred && (
          <Fact k="Transferred to" v={call.transfer_destination ?? 'somewhere Vapi did not name'} />
        )}
      </dl>

      {/* Vapi's own words about the call, kept apart from the call's own. A summary it
          did not write is not an empty summary, so the section is simply not here. */}
      {call.summary && (
        <section>
          <h3>Summary</h3>
          <p className="prose">{call.summary}</p>
        </section>
      )}

      {call.success_eval && (
        <section>
          <h3>Success evaluation</h3>
          {/* Whatever rubric the assistant was given — "true", "8", a sentence. Shown as
              it came, with no verdict painted on top of it. */}
          <p className="prose">{call.success_eval}</p>
        </section>
      )}

      {call.recording_url && (
        <section>
          <h3>Recording</h3>
          <p>
            <a href={call.recording_url} target="_blank" rel="noreferrer noopener">
              Open the recording at Vapi ↗
            </a>
          </p>
          <p className="hint">graphify stores the link and never the audio.</p>
        </section>
      )}

      <section>
        <h3>Tool calls</h3>
        {call.tool_call_rows.length === 0 ? (
          <p className="hint">This call made no tool calls.</p>
        ) : (
          <ul className="tools">
            {call.tool_call_rows.map((tool, i) => (
              // Nothing on a tool call is unique — the same tool at the same second is a
              // real thing to log twice — so the position in the list is the key.
              <li key={i} className={tool.failed ? 'failed' : undefined}>
                <p className="head">
                  <b>{tool.name ?? DASH}</b>
                  <span className="hint">{at(tool.seconds_from_start)}</span>
                  {/* The word, not a colour: a reader who cannot see the one still reads
                      the other. */}
                  {tool.failed && <span className="badge">failed</span>}
                </p>
                {tool.arguments && <pre className="mono">{tool.arguments}</pre>}
                {tool.result_excerpt && (
                  <pre className="mono">
                    {tool.result_excerpt}
                    <span className="hint"> (excerpt)</span>
                  </pre>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h3>Transcript</h3>
        {call.transcript ? (
          <ol className="transcript">
            {turns(call.transcript).map((turn, i) => (
              <li key={i} className={turn.speaker?.toLowerCase() === 'user' ? 'user' : undefined}>
                <span className="who">{turn.speaker ?? DASH}</span>
                <span className="said">{turn.text}</span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="hint">Vapi returned no transcript for this call.</p>
        )}
      </section>
    </>
  )
}

function Fact({ k, v }: { k: string; v: ReactNode }) {
  return (
    <>
      <dt>{k}</dt>
      <dd>{v}</dd>
    </>
  )
}
