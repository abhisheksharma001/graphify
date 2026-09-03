// The engine's API, as the browser sees it.
//
// Every type here is a view of the JSON the engine returns, narrowed to the fields this
// dashboard reads. The engine sends more; a field absent from a type is one nothing has
// needed yet, not one that is missing from the response.
//
// Nothing in this file talks to Vapi. The browser never holds a key: `/api` is the only
// origin it knows, and the engine is the only thing that ever calls Vapi.

/** A 401. Distinct from any other failure because it has its own answer: sign in. */
export class Unauthorized extends Error {}

export type Org = {
  id: number
  name: string
}

/** `/api/assistants` returns far more per row; the picker needs a name and an id. */
export type Assistant = {
  id: string
  name: string | null
}

/** Counts are counts, so zero is an answer. Everything measured is nullable, because an
 * hour that priced nothing has no cost — it does not have a cost of zero. */
export type Totals = {
  calls: number
  cost: number | null
  duration_avg: number | null
  latency_p50: number | null
  latency_p95: number | null
}

export type Bucket = Totals & { bucket: string }

export type Stats = {
  by_ended_group: Record<string, number>
  by_ended_reason: Record<string, number>
  /** Every bucket across the span, including the empty ones. */
  per_bucket: Bucket[]
  /** `1h` or `1d`, so the axis can be labelled without guessing. */
  bucket_size: string
  totals: Totals
}

async function reason(res: Response): Promise<string> {
  // The engine puts its message in `{"error": ...}`. Anything else — a proxy, a crash
  // page — gets reported by status, which is at least true.
  try {
    const body = (await res.json()) as { error?: string }
    if (typeof body.error === 'string') return body.error
  } catch {
    /* not JSON */
  }
  return `${res.status} ${res.statusText}`
}

async function get<T>(path: string, params?: URLSearchParams): Promise<T> {
  const query = params?.toString()
  const res = await fetch(query ? `${path}?${query}` : path, {
    headers: { accept: 'application/json' },
  })
  if (res.status === 401) throw new Unauthorized(await reason(res))
  if (!res.ok) throw new Error(await reason(res))
  return (await res.json()) as T
}

export const orgs = () => get<Org[]>('/api/orgs')

export const assistants = (org: number | null) =>
  get<Assistant[]>('/api/assistants', new URLSearchParams(org == null ? {} : { org: String(org) }))

/** One row of `/api/calls`, narrowed to what the ended-group chart reads. */
export type Call = {
  id: string
  created_at: string | null
  ended_reason: string | null
  ended_group: string | null
}

export const stats = (params: URLSearchParams) => get<Stats>('/api/stats', params)

export const calls = (params: URLSearchParams) => get<Call[]>('/api/calls', params)

/** The session cookie comes back on the response and is `HttpOnly`; there is nothing to
 * store here, and deliberately nothing this code could read. */
export async function login(password: string): Promise<void> {
  const res = await fetch('/api/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ password }),
  })
  if (!res.ok) throw new Error(await reason(res))
}
