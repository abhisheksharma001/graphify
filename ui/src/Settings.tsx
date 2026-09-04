// Orgs, keys and retention.
//
// This is the only screen that sends a key anywhere, and the rules it works under are the
// reason it looks the way it does. A key goes up and a *status* comes back — a name, a
// flag and four characters — so there is no response on this page that could carry a key
// even if something asked for one. The field a key is typed into is uncontrolled and is
// cleared before the request leaves, so the value lives in one local variable for the
// length of one `await` and is in neither React state nor the DOM afterwards.
//
// The browser never talks to Vapi. "Test connection" asks the engine to make one `GET`
// with the key it already holds, and reads the verdict.

import { useEffect, useRef, useState } from 'react'
import * as api from './api'
import { Unauthorized } from './api'
import type { Org, SecretStatus } from './api'

/** The engine's cap on retention, D-5. Stated here too so the field can refuse a number
 * before it is sent, rather than only reporting the engine's refusal afterwards. */
const MAX_KEEP_DAYS = 14

/** Provider names as people write them, not as the API spells them. */
const LABELS: Record<string, string> = {
  vapi: 'Vapi',
  anthropic: 'Anthropic',
  openai: 'OpenAI',
}

const label = (name: string) => LABELS[name] ?? name

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** What a stored key is allowed to look like: the word, and the tail if there is one. A
 * value too short for a tail is still set, and says so. */
function Stored({ status }: { status: SecretStatus }) {
  if (!status.set) return <span className="hint">not set</span>
  return (
    <span className="stored">
      set{status.last4 && <> · ••••{status.last4}</>}
    </span>
  )
}

/** One key: what is stored, and a field to replace it.
 *
 * `save` returns the whole new status list, because that is what the engine answers with
 * — the caller writes it back rather than assuming what landed. */
function KeyField({
  status,
  hint,
  save,
  onError,
}: {
  status: SecretStatus
  hint: string
  save: (value: string) => Promise<SecretStatus[]>
  onError: (e: unknown) => void
}) {
  const field = useRef<HTMLInputElement>(null)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    const el = field.current
    const value = el?.value.trim()
    if (!el || !value) return
    // Cleared before the request goes out, not after it comes back. A key sitting in an
    // input while the network is slow is a key on screen, and a failed save must not
    // leave one there either — retyping it is the cheaper half of that trade.
    el.value = ''
    setBusy(true)
    setFailed(null)
    try {
      await save(value)
    } catch (err) {
      if (err instanceof Unauthorized) onError(err)
      else setFailed(message(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="key" onSubmit={submit}>
      <label htmlFor={`key-${status.name}`}>{label(status.name)} key</label>
      <Stored status={status} />
      <input
        id={`key-${status.name}`}
        type="password"
        ref={field}
        autoComplete="off"
        placeholder={status.set ? 'Replace it' : 'Paste it'}
      />
      <button type="submit" disabled={busy}>
        {busy ? 'Saving…' : 'Save'}
      </button>
      <p className="hint span">{hint}</p>
      {failed && <p className="error span">{failed}</p>}
    </form>
  )
}

/** Retention, per org. Both fields go up together: they are both nullable, so a request
 * that carried only one could not tell "leave it" from "clear it". */
function Limits({
  org,
  onSaved,
  onError,
}: {
  org: Org
  onSaved: () => void
  onError: (e: unknown) => void
}) {
  const [days, setDays] = useState(String(org.keep_days ?? MAX_KEEP_DAYS))
  const [max, setMax] = useState(org.max_calls === null ? '' : String(org.max_calls))
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true)
    setFailed(null)
    setSaved(false)
    try {
      // An empty box is "no limit", which is a setting and not a blank. Days always has
      // a number, because there is no such thing as keeping calls forever.
      await api.saveLimits(org.id, Number(days), max.trim() === '' ? null : Number(max))
      onSaved()
      setSaved(true)
    } catch (err) {
      if (err instanceof Unauthorized) onError(err)
      else setFailed(message(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="limits" onSubmit={submit}>
      <label htmlFor={`days-${org.id}`}>Keep for</label>
      <span className="unit">
        <input
          id={`days-${org.id}`}
          type="number"
          min={1}
          max={MAX_KEEP_DAYS}
          required
          value={days}
          onChange={(e) => setDays(e.target.value)}
        />
        days
      </span>
      <label htmlFor={`max-${org.id}`}>At most</label>
      <span className="unit">
        <input
          id={`max-${org.id}`}
          type="number"
          min={1}
          placeholder="no limit"
          value={max}
          onChange={(e) => setMax(e.target.value)}
        />
        calls
      </span>
      <button type="submit" disabled={busy}>
        {busy ? 'Saving…' : 'Save'}
      </button>
      <p className="hint span">
        {MAX_KEEP_DAYS} days is the cap and cannot be raised here. Whatever falls outside
        either limit is deleted on the next sync.
      </p>
      {saved && <p className="hint span">Saved.</p>}
      {failed && <p className="error span">{failed}</p>}
    </form>
  )
}

/** Ask the engine to spend one `GET` on the org's key and say what came back. */
function TestKey({ org, onError }: { org: number; onError: (e: unknown) => void }) {
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<string | null>(null)
  const [ok, setOk] = useState(false)

  async function run() {
    setBusy(true)
    setResult(null)
    try {
      const answer = await api.testOrg(org)
      setOk(answer.ok)
      // A key that does not work is an answer, so it is reported in the same place and
      // in the same words as one that does — with the verdict written out, never left to
      // a colour to carry.
      setResult(answer.ok ? `Vapi returned ${answer.assistants} assistants.` : answer.error)
    } catch (err) {
      if (err instanceof Unauthorized) onError(err)
      else {
        setOk(false)
        setResult(message(err))
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <p className="test">
      <button type="button" onClick={run} disabled={busy}>
        {busy ? 'Testing…' : 'Test connection'}
      </button>
      {result && (
        <span className={ok ? 'verdict ok' : 'verdict bad'}>
          <b>{ok ? 'Works' : 'Failed'}</b> {result}
        </span>
      )}
    </p>
  )
}

/** One org: its key, its connectivity, its retention. */
function OrgCard({
  org,
  onSaved,
  onError,
}: {
  org: Org
  onSaved: () => void
  onError: (e: unknown) => void
}) {
  const [keys, setKeys] = useState<SecretStatus[] | null>(null)

  useEffect(() => {
    let live = true
    api
      .secrets(org.id)
      .then((list) => live && setKeys(list))
      .catch((e: unknown) => live && onError(e))
    return () => {
      live = false
    }
  }, [org.id, onError])

  return (
    <section className="card org">
      <h3>{org.name}</h3>
      {keys === null ? (
        <p className="notice">Loading…</p>
      ) : (
        keys.map((status) => (
          <KeyField
            key={status.name}
            status={status}
            hint="Stored encrypted. It is sent to Vapi by the engine and is never returned to this page."
            save={(value) =>
              api.setSecret(org.id, status.name, value).then((list) => {
                setKeys(list)
                return list
              })
            }
            onError={onError}
          />
        ))
      )}
      <TestKey org={org.id} onError={onError} />
      <Limits org={org} onSaved={onSaved} onError={onError} />
    </section>
  )
}

/** The "+" flow, in one submit: make the org, store its key, then test it. Three requests
 * because they are three things, and the order is what makes the last one meaningful. */
function AddOrg({ onAdded, onError }: { onAdded: () => void; onError: (e: unknown) => void }) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const field = useRef<HTMLInputElement>(null)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    const el = field.current
    const key = el?.value.trim() ?? ''
    if (el) el.value = ''
    setBusy(true)
    setFailed(null)
    setNote(null)
    try {
      const org = await api.createOrg(name.trim())
      if (key) {
        await api.setSecret(org.id, 'vapi', key)
        const answer = await api.testOrg(org.id)
        // The org exists either way — a key that does not work is worth saying out loud,
        // but it is not a reason to have refused to create the org.
        setNote(answer.ok ? `Key works: ${answer.assistants} assistants.` : `Key failed: ${answer.error}`)
      }
      setName('')
      setOpen(false)
      onAdded()
    } catch (err) {
      if (err instanceof Unauthorized) onError(err)
      else setFailed(message(err))
    } finally {
      setBusy(false)
    }
  }

  if (!open)
    return (
      <p className="add">
        <button type="button" onClick={() => setOpen(true)}>
          + Add an org
        </button>
        {note && <span className="hint"> {note}</span>}
      </p>
    )

  return (
    <form className="card add-org" onSubmit={submit}>
      <h3>New org</h3>
      <label htmlFor="new-name">Name</label>
      <input
        id="new-name"
        autoFocus
        required
        value={name}
        onChange={(e) => setName(e.target.value)}
      />
      <label htmlFor="new-key">Vapi key</label>
      <input id="new-key" type="password" ref={field} autoComplete="off" placeholder="Optional" />
      <p className="hint span">
        With a key, the org is created, the key is stored, and the connection is tested —
        in that order. Without one, the org is created and the key can be added below.
      </p>
      <p className="span row">
        <button type="submit" disabled={busy}>
          {busy ? 'Adding…' : 'Add'}
        </button>
        <button type="button" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </p>
      {failed && <p className="error span">{failed}</p>}
    </form>
  )
}

/** The model keys. One account pays for them and every org's calls spend them, so they
 * belong to the install and are stored once, under no org. */
function ModelKeys({ onError }: { onError: (e: unknown) => void }) {
  const [keys, setKeys] = useState<SecretStatus[] | null>(null)

  useEffect(() => {
    let live = true
    api
      .globalSecrets()
      .then((list) => live && setKeys(list))
      .catch((e: unknown) => live && onError(e))
    return () => {
      live = false
    }
  }, [onError])

  return (
    <section className="card org">
      <h3>Model keys</h3>
      <p className="sub">
        Used by the brain, not by any org. Nothing here calls a model on its own: every
        run shows its cost and waits to be told to go.
      </p>
      {keys === null ? (
        <p className="notice">Loading…</p>
      ) : (
        keys.map((status) => (
          <KeyField
            key={status.name}
            status={status}
            hint="Stored encrypted, once, for the whole install."
            save={(value) =>
              api.setGlobalSecret(status.name, value).then((list) => {
                setKeys(list)
                return list
              })
            }
            onError={onError}
          />
        ))
      )}
    </section>
  )
}

export default function Settings({
  orgs,
  onOrgs,
  onError,
}: {
  orgs: Org[]
  onOrgs: () => void
  onError: (e: unknown) => void
}) {
  return (
    <div className="settings">
      <section className="card">
        <h2>Settings</h2>
        <p className="sub">
          Keys are stored encrypted and are never sent back to this page — a saved key
          shows as its last four characters and nothing more.
        </p>
      </section>

      {orgs.map((org) => (
        // Keyed by id, so the fields of one org are never the fields of another with the
        // values swapped underneath them.
        <OrgCard key={org.id} org={org} onSaved={onOrgs} onError={onError} />
      ))}

      <AddOrg onAdded={onOrgs} onError={onError} />
      <ModelKeys onError={onError} />
    </div>
  )
}
