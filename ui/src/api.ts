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
  tool_failures: number | null
  transfers: number | null
  cost: number | null
  /** What the cost was spent on. These add up to `cost`, so they stack under it. */
  cost_stt: number | null
  cost_llm: number | null
  cost_tts: number | null
  cost_vapi: number | null
  cost_transport: number | null
  cost_analysis: number | null
  prompt_tokens: number | null
  completion_tokens: number | null
  cached_tokens: number | null
  /** Turn latency and what it was spent waiting on, in ms. The components are averages
   * over the calls that reported them, so they add up to `latency_avg` — not to the
   * percentiles, which are a different question about the same calls. */
  latency_avg: number | null
  latency_model: number | null
  latency_voice: number | null
  latency_transcriber: number | null
  latency_endpointing: number | null
  duration_avg: number | null
  latency_p50: number | null
  latency_p95: number | null
}

/** One row of `by_assistant`. The engine leaves the breakdowns NULL here — they are not
 * grouped per assistant — so this row is honest about carrying counts, cost and duration
 * and nothing else. */
export type ByAssistant = Totals & {
  assistant_id: string | null
  name: string | null
}

export type Bucket = Totals & { bucket: string }

export type Stats = {
  by_ended_group: Record<string, number>
  by_ended_reason: Record<string, number>
  /** Every bucket across the span, including the empty ones. */
  per_bucket: Bucket[]
  /** `1h` or `1d`, so the axis can be labelled without guessing. */
  bucket_size: string
  /** Failed tool calls by tool name, over the whole selection. */
  tool_failures_by_name: Record<string, number>
  by_assistant: ByAssistant[]
  /** Vapi's `successEvaluation`, counted. A call it did not evaluate is not in here at
   * all — "no verdict" is not a verdict, and must not stand beside the real ones. */
  success_eval_counts: Record<string, number>
  structured_fields: StructuredField[]
  totals: Totals
}

/** One bucket of a numeric structured key, on the same axis as `per_bucket`. */
export type NumberBucket = { bucket: string; avg: number | null }

/** One top-level key of `analysis.structuredData`, with the one chart it can honestly
 * carry. The engine decides which: a key is a `number` only when every value it carried
 * was one, `text` when they were all scalars, and `other` when any was an object or a
 * list — which is neither a count nor an average. */
export type StructuredField = {
  key: string
  kind: 'text' | 'number' | 'other'
  /** Calls that carried a non-null value for this key. */
  calls: number
  /** `text` only: value → calls, already folded to what a chart shows. */
  counts: Record<string, number>
  /** `text` only: the values the fold left out, summed by the engine over all of them. */
  tail: { values: number; calls: number } | null
  /** `number` only. */
  per_bucket: NumberBucket[]
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
