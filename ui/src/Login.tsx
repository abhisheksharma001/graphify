// The password gate, when the engine was started with one.
//
// The password goes straight to `/api/login` and is never kept: the engine answers with an
// `HttpOnly` cookie, which this code could not read even if it wanted to.

import { useState } from 'react'
import * as api from './api'

export default function Login({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.login(password)
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="login" onSubmit={submit}>
      <h1 className="wordmark">graphify</h1>
      <label htmlFor="password">Password</label>
      <input
        id="password"
        type="password"
        autoFocus
        value={password}
        onChange={(e) => setPassword(e.target.value)}
      />
      <button type="submit" disabled={busy}>
        {busy ? 'Signing in…' : 'Sign in'}
      </button>
      {error && <p className="error">{error}</p>}
    </form>
  )
}
