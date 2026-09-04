// The three things the analyst owns on a saved pattern, and the button that re-counts it.
//
// The rule is edited as the JSON it is. That is deliberate: D-2 says a pattern is data and
// never code, and the honest way to show data is to show it. The DSL is small enough to
// read — phrases, regexes, a speaker, and a handful of structural conditions — and the
// engine refuses one it would choke on later, naming the key it choked on, so a rule that
// saves is a rule that will still run at three in the morning with nobody watching.
//
// Two checks, in two places, for two different mistakes. A missing brace is caught here as
// it is typed, because that answer needs no server. Whether the JSON is a *rule* is the
// engine's to say, and it says so on save, having stored nothing.
//
// **Nothing on this panel spends.** Re-apply runs the rule over the stored calls, which is
// arithmetic — in `free`, in `hybrid` and in `full` alike. The mode and the cap say what
// tomorrow's sync may spend; they do not make this button cost anything today.

import { useState } from 'react'
import * as api from '../api'
import { MODES } from '../api'
import type { Mode, Pattern } from '../api'

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** The mode a row with no mode is in. The engine's column default says the same. */
const DEFAULT_MODE: Mode = 'free'

/** Dollars a day, when the stored row names none. Matches the engine's column default. */
const DEFAULT_CAP = '1.00'

const isMode = (m: string | null): m is Mode => MODES.some((known) => known === m)

/** The draft as JSON, or the reason it is not. An empty box is a rule of `null`, which is
 * what a pattern being re-learned looks like between the two halves of the wizard — not a
 * rule that matches nothing, and not an error. */
function parse(text: string): { rule: unknown; error: null } | { rule: null; error: string } {
  const trimmed = text.trim()
  if (trimmed === '') return { rule: null, error: null }
  try {
    return { rule: JSON.parse(trimmed) as unknown, error: null }
  } catch (e) {
    return { rule: null, error: message(e) }
  }
}

const asText = (rule: unknown) => (rule == null ? '' : JSON.stringify(rule, null, 2))

export default function Editor({
  pattern,
  onChanged,
}: {
  pattern: Pattern
  /** A save or a re-apply changed something the screen around this is drawing. */
  onChanged: () => void
}) {
  // Seeded once. The panel is keyed by pattern id above, so a different pattern is a
  // different editor; a reload of the same one must not throw away what is being typed.
  const [text, setText] = useState(() => asText(pattern.rule))
  const [mode, setMode] = useState<Mode>(() =>
    isMode(pattern.mode) ? pattern.mode : DEFAULT_MODE,
  )
  const [cap, setCap] = useState(() =>
    pattern.daily_cap_usd == null ? DEFAULT_CAP : String(pattern.daily_cap_usd),
  )

  const [busy, setBusy] = useState<string | null>(null)
  const [refused, setRefused] = useState<string | null>(null)
  /** What the last re-apply counted. Null until one has been run from here. */
  const [applied, setApplied] = useState<{ matched: number; of: number } | null>(null)
  /** Saved, and not re-applied since. The stored matches are still the old rule's, so
   * every count on the screen is about a rule that is no longer the one stored. */
  const [unapplied, setUnapplied] = useState(false)

  const draft = parse(text)
  const dollars = Number(cap)
  const capOk = cap.trim() !== '' && Number.isFinite(dollars) && dollars > 0
  const ready = draft.error === null && capOk && busy === null

  const during = async (what: string, work: () => Promise<void>) => {
    setBusy(what)
    setRefused(null)
    try {
      await work()
    } catch (e) {
      setRefused(message(e))
    } finally {
      setBusy(null)
    }
  }

  const save = () =>
    during('Checking the rule…', async () => {
      if (draft.error !== null) return
      await api.savePattern(pattern.id, { rule: draft.rule, mode, daily_cap_usd: dollars })
      setUnapplied(true)
      setApplied(null)
      onChanged()
    })

  const reapply = () =>
    during('Re-counting…', async () => {
      const result = await api.applyPattern(pattern.id)
      setApplied(result)
      setUnapplied(false)
      onChanged()
    })

  return (
    <section className="card editor">
      <h2>Rule</h2>
      <p className="sub">
        The pattern as data, not code. The engine checks it against the DSL before it stores
        it, and refuses one it would choke on later.
      </p>

      <textarea
        className="mono rule-edit"
        rows={12}
        spellCheck={false}
        value={text}
        onChange={(e) => setText(e.target.value)}
        aria-label="Rule JSON"
      />
      {/* Said as it is typed, and said as what it is: not JSON yet, which is a different
          complaint from "not a rule". */}
      {draft.error !== null && <p className="error">Not JSON yet: {draft.error}</p>}
      {draft.error === null && draft.rule === null && (
        <p className="hint">
          Empty. Saving this clears the rule, and the pattern counts nothing until it has
          another.
        </p>
      )}

      <div className="settings">
        <div className="field">
          <label htmlFor="pattern-mode">Mode</label>
          <select
            id="pattern-mode"
            value={mode}
            onChange={(e) => setMode(e.target.value as Mode)}
          >
            {MODES.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>

        <div className="field">
          <label htmlFor="pattern-cap">Daily cap</label>
          <input
            id="pattern-cap"
            type="number"
            min="0.01"
            step="0.01"
            value={cap}
            onChange={(e) => setCap(e.target.value)}
          />
        </div>
      </div>
      <p className="hint">
        {mode === 'free'
          ? 'free: the rule alone decides, and no model ever reads a call. The cap below is stored for the day the mode changes.'
          : mode === 'hybrid'
            ? 'hybrid: the rule picks the candidates and a model confirms the new ones, up to the cap each day.'
            : 'full: a model reads every new call, up to the cap each day.'}
      </p>
      {!capOk && <p className="error">The cap is dollars a day, and has to be more than nothing.</p>}

      <div className="actions">
        <button type="button" onClick={save} disabled={!ready}>
          Save rule
        </button>
        <button type="button" onClick={reapply} disabled={busy !== null}>
          Re-apply
        </button>
      </div>
      <p className="hint">
        Re-apply runs the rule over the calls already stored. It costs nothing in any mode —
        the rule half is arithmetic, and no model reads anything.
      </p>

      {unapplied && (
        <p className="hint">
          Saved. Every count on this screen is still the last run&rsquo;s: re-apply to
          recount them against the rule that is now stored.
        </p>
      )}
      {applied !== null && (
        <p className="hint">
          Matched {applied.matched} of {applied.of} calls in the org.
        </p>
      )}
      {busy !== null && <p className="hint">{busy}</p>}
      {refused !== null && <p className="error">{refused}</p>}
    </section>
  )
}
