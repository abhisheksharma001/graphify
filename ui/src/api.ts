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
  /** Retention. `keep_days` is capped at 14 by the engine; `max_calls` is null for no
   * limit at all, which is a real setting and not a missing one. */
  keep_days: number | null
  max_calls: number | null
}

/** What the settings screen is allowed to know about a key: that there is one, and its
 * last four characters. There is no shape of this type that could carry the value. */
export type SecretStatus = {
  name: string
  set: boolean
  last4: string | null
}

/** What `POST /api/orgs/{id}/test` answers. A key that does not work is an answer, not a
 * failure — the request succeeded and told us the truth. */
export type TestResult = { ok: true; assistants: number } | { ok: false; error: string }

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

/** One chart of the dashboard, and whether it is drawn. The order of the list is the
 * order the charts appear in.
 *
 * Ids, not titles: rewording a chart must not turn it back on for a reader who had turned
 * it off. */
export type ChartPref = { id: string; on: boolean }

/** An empty list means nothing has been saved yet, and is read as "draw everything the
 * dashboard has" — never as "draw nothing". */
export type Layout = { charts: ChartPref[] }

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

async function send<T>(method: string, path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: { 'content-type': 'application/json', accept: 'application/json' },
    body: JSON.stringify(body),
  })
  if (res.status === 401) throw new Unauthorized(await reason(res))
  if (!res.ok) throw new Error(await reason(res))
  return (await res.json()) as T
}

const put = <T>(path: string, params: URLSearchParams, body: unknown) =>
  send<T>('PUT', `${path}?${params}`, body)

export const orgs = () => get<Org[]>('/api/orgs')

export const createOrg = (name: string) => send<Org>('POST', '/api/orgs', { name })

/** Both limits go every time. They are both nullable, so "leave this one alone" and
 * "clear this one" would otherwise be the same request. */
export const saveLimits = (org: number, keep_days: number | null, max_calls: number | null) =>
  send<Org>('PUT', `/api/orgs/${org}`, { keep_days, max_calls })

/** The org's own keys — the Vapi key. The response is the new status, never the value. */
export const secrets = (org: number) => get<SecretStatus[]>(`/api/orgs/${org}/secrets`)

export const setSecret = (org: number, name: string, value: string) =>
  send<SecretStatus[]>('PUT', `/api/orgs/${org}/secrets/${encodeURIComponent(name)}`, {
    value,
  })

/** The install's own keys — the model providers, which no client org owns. */
export const globalSecrets = () => get<SecretStatus[]>('/api/secrets')

export const setGlobalSecret = (name: string, value: string) =>
  send<SecretStatus[]>('PUT', `/api/secrets/${encodeURIComponent(name)}`, { value })

/** One `GET /assistant` at Vapi with the org's stored key, to answer "is this key any
 * good". The engine makes the call; the browser never holds the key. */
export const testOrg = (org: number) => send<TestResult>('POST', `/api/orgs/${org}/test`, null)

export const assistants = (org: number | null) =>
  get<Assistant[]>('/api/assistants', new URLSearchParams(org == null ? {} : { org: String(org) }))

/** One row of `/api/calls`. Every measure is nullable, because a call Vapi never
 * priced has no cost — not a cost of zero — and the table has to be able to say so. */
export type Call = {
  id: string
  created_at: string | null
  assistant_id: string | null
  assistant_name: string | null
  duration_s: number | null
  ended_reason: string | null
  ended_group: string | null
  cost: number | null
  transferred: boolean | null
  tool_calls: number | null
  tool_failures: number | null
  turns: number | null
  lat_turn_p50_ms: number | null
  lat_turn_p95_ms: number | null
  success_eval: string | null
  summary: string | null
  /** Why a model said this call is one of a pattern's, in its own words. Only ever set
   * when the request named a pattern, and null even then for a call the model never read:
   * a rule matches every call it matches, and only the sample was labelled. */
  evidence: string | null
}

/** One tool invocation on a call. `result_excerpt` is an excerpt: the engine stores a
 * prefix, not the whole result, so nothing here is the full payload. */
export type ToolCall = {
  name: string | null
  seconds_from_start: number | null
  failed: boolean | null
  arguments: string | null
  result_excerpt: string | null
}

/** `/api/calls/{id}`: the row again, plus everything only the drawer reads.
 *
 * `recording_url` is a URL and nothing else. D-3: the audio is never downloaded and
 * never stored, so the drawer links out to it and never plays it. */
export type CallDetail = Call & {
  status: string | null
  call_type: string | null
  started_at: string | null
  ended_at: string | null
  transfer_destination: string | null
  lat_turn_avg_ms: number | null
  transcript: string | null
  recording_url: string | null
  tool_call_rows: ToolCall[]
}

export const stats = (params: URLSearchParams) => get<Stats>('/api/stats', params)

export const calls = (params: URLSearchParams) => get<Call[]>('/api/calls', params)

export const call = (id: string) => get<CallDetail>(`/api/calls/${encodeURIComponent(id)}`)

const forOrg = (org: number) => new URLSearchParams({ org: String(org) })

export const dashboard = (org: number) => get<Layout>('/api/dashboard', forOrg(org))

/** The engine answers with what it stored, so the caller never has to assume it landed. */
export const saveDashboard = (org: number, charts: ChartPref[]) =>
  put<Layout>('/api/dashboard', forOrg(org), { charts })

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

// --- patterns: the wizard's half of the engine ------------------------------------------
//
// Everything below starts a brain job and then watches it. The engine answers a start with
// an id and nothing else; where the job got to, what it quoted and what it cost are all
// read back through `job`. Nothing here spends: `go` is the one call in this file that lets
// a model read a call, and it is sent by one click on a button that already shows a price.

/** One row of a plan: a condition, and what it does to the count.
 *
 * `if_`, not `if`. The name is the same in BAML, the brain, the engine and here, because
 * a rename that only one hop can see is a rename that goes wrong once, quietly. */
export type PlanRow = { if_: string; then: string }

/** What the model understood, in a form the analyst can disagree with. */
export type Plan = {
  rows: PlanRow[]
  /** What it would have to ask to be sure. Empty when it is. Never more than three. */
  questions: string[]
  /** 0 to 1. The wizard's gate: under it, nothing may be read. */
  confidence: number
  /** False when a row cannot be checked by the rule DSL — which is fatal whatever the
   * confidence, because the rule is what counts the calls for free afterwards. */
  expressible: boolean
  reason: string
}

/** Where a job has got to.
 *
 * `waiting` is the interesting one: the brain has printed its price and is parked with
 * its stdin open, having read nothing and spent nothing, until it is told to go. */
export type JobStatus = 'running' | 'waiting' | 'done' | 'failed' | 'expired'

export type Job = {
  id: number
  kind: string
  status: JobStatus
  /** Null until the job has reported any. A job that has said nothing has no progress —
   * it does not have a progress of zero. */
  progress: { done: number; of: number } | null
  /** What the brain quoted, in dollars. A ceiling, not a forecast: output is priced at
   * the ceiling a batch cannot exceed, so short calls quote several times their cost. */
  estimate_usd: number | null
  /** What it actually cost. Null until the job is over. */
  cost_usd: number | null
  /** The brain's last line, parsed. `null` while the job is still running. */
  output: unknown
  /** Everything the brain wrote to stderr, keys already scrubbed by the engine. */
  log: string
  created_at: string
  finished_at: string | null
}

/** What `label` answers with. Every id asked about is in exactly one of the four lists,
 * and each list has exactly one cause. */
export type Labelled = {
  labels: { call_id: string; match: boolean; evidence?: string }[]
  no_transcript: string[]
  no_label: string[]
  not_reached: string[]
  /** What the run actually cost — the real figure, against the estimate's ceiling. */
  usd: number
  batches: number
  model: string
  /** `"declined"` when nobody said go, `"cap"` when the cap stopped it. Null when it ran
   * to the end. */
  stopped: string | null
}

/** What `synthesize` answers with. It has already written the `patterns` row by the time
 * this arrives — `pattern_id` is that row. */
export type Synthesized = {
  pattern_id: number
  rule: unknown
  chart: { kind: string; title: string }
  /** 0 to 1: how much of the sample the rule and the model agree about. */
  agreement: number
  agreed: number
  of: number
  matched_by_rule: number
  matched_by_model: number
  refined: boolean
  reason: string | null
  usd: number
  model: string
}

/** The engine's answer to a start: the row exists and the child is coming up. */
type Started = { id: number; status: JobStatus }

const startJob = (fn: string, org: number, body: unknown) =>
  send<Started>('POST', `/api/patterns/${fn}?${forOrg(org)}`, body)

export const startPlan = (org: number, body: { criterion: string; system_prompt?: string }) =>
  startJob('plan', org, body)

export const startClarify = (
  org: number,
  body: { criterion: string; plan: Plan; answers: { question: string; answer: string }[] },
) => startJob('clarify', org, body)

export const startLabel = (
  org: number,
  body: { criterion: string; plan: Plan; call_ids: string[]; model: string; max_usd: number },
) => startJob('label', org, body)

export const startSynthesize = (
  org: number,
  body: {
    criterion: string
    plan: Plan
    labels: Labelled['labels']
    model: string
    max_usd: number
    org_id: number
    name: string
    assistant_ids?: string[]
  },
) => startJob('synthesize', org, body)

export const job = (id: number) => get<Job>(`/api/jobs/${id}`)

/** The go. This is the only call in this file that lets a model read a call, and the
 * button that sends it carries the price the brain quoted. */
export const go = (id: number) => send<Started>('POST', `/api/jobs/${id}/go`, null)

/** One assistant's system prompt. Not in `assistants` above, which is a picker: these run
 * to tens of kilobytes each, and the wizard asks for the one it is planning against only
 * when the analyst has ticked the box. */
export const assistantPrompt = (org: number, id: string) =>
  get<{ id: string; system_prompt: string | null }>(
    `/api/assistants/${encodeURIComponent(id)}/prompt`,
    forOrg(org),
  )

// --- the ask box --------------------------------------------------------------------------

/** What a question would cost, and what it would be answered from.
 *
 * Asking for one of these starts nothing. Everywhere else in graphify a price comes from
 * the brain, which means a job row and a parked interpreter before anyone has seen a
 * figure; here the engine works it out on the request, so reading a price and walking away
 * leaves nothing behind. That is what makes `Cancel` below a button that sends nothing. */
export type AskQuote = {
  question: string
  model: string
  /** The calls whose transcripts would go in, shortest first. */
  call_ids: string[]
  /** How many the sample held before the token cap trimmed it. `call_ids.length` short of
   * this means the context filled up and the answer rests on fewer calls. */
  readable: number
  tokens_in: number
  usd: number
}

/** What `ask` answers with. `answer` is Markdown, in the small subset `Answer.tsx` draws —
 * null when the run was stopped, which is the only time `stopped` is set. */
export type Answered = {
  answer: string | null
  calls: string[]
  no_transcript: string[]
  usd: number
  model: string
  stopped: string | null
}

/** Price a question over the current selection. Creates nothing. */
export const askQuote = (params: URLSearchParams, body: { question: string; model: string }) =>
  send<AskQuote>('POST', `/api/ask/quote?${params}`, body)

/** Ask it. `max_usd` is the figure that was quoted and approved; the engine prices the
 * question again and refuses rather than going over it. */
export const startAsk = (
  params: URLSearchParams,
  body: { question: string; model: string; max_usd: number },
) => send<Started>('POST', `/api/ask?${params}`, body)

// --- patterns: the ones already saved ----------------------------------------------------
//
// Nothing below spends. A rule is the free half of a pattern — the engine runs it over the
// stored calls and that is arithmetic — so re-applying one costs nothing in any mode.

/** The models the brain accepts, spelled the way it spells them — `CLIENTS` in
 * `graphify_brain/label.py`. Sonnet first: it is the one the walkthrough uses. */
export const MODELS = ['sonnet', 'opus', 'gpt'] as const

export type Model = (typeof MODELS)[number]

/** D-8's three. `free` puts no model in the loop at all; the other two do, which is why
 * the cap beside them is required rather than optional. */
export const MODES = ['free', 'hybrid', 'full'] as const

export type Mode = (typeof MODES)[number]

/** What the brain suggested this pattern be drawn as. `kind` is `Line` or `Bar` in BAML's
 * spelling; anything else is a suggestion this dashboard does not have. */
export type ChartSuggestion = { kind: string; title: string }

/** A saved pattern. The four JSON columns arrive parsed; one the engine could not parse
 * comes back null rather than taking the row with it, so every one of them is nullable. */
export type Pattern = {
  id: number
  org_id: number | null
  name: string | null
  criterion: string | null
  assistant_ids: string[] | null
  plan: Plan | null
  rule: unknown
  chart: ChartSuggestion | null
  model: string | null
  /** Null on a half-written row. The editor reads that as `free`, which is what the
   * engine's own column default says. */
  mode: string | null
  daily_cap_usd: number | null
  sample_size: number | null
  agreement: number | null
  created_at: string | null
  /** How many calls of the current selection this pattern matched. Null when no selection
   * was named — an edit's answer is about a row, not about a window of calls. */
  matched: number | null
}

/** The whole filter set, not just the org: `matched` is a count of the calls on screen. */
export const patterns = (params: URLSearchParams) => get<Pattern[]>('/api/patterns', params)

/** All three go every time. They are what the analyst owns on a saved pattern, and sending
 * only the changed one would make "leave this alone" and "clear this" the same request.
 *
 * The engine validates the rule against the DSL and refuses to store one it would choke on
 * later, so a 400 here is the rule being wrong and says which key. */
export const savePattern = (
  id: number,
  body: { rule: unknown; mode: Mode; daily_cap_usd: number },
) => send<Pattern>('PUT', `/api/patterns/${id}`, body)

/** Re-run the rule over the org's calls. Free in every mode: this is the rule half. */
export const applyPattern = (id: number) =>
  send<{ matched: number; of: number }>('POST', `/api/patterns/${id}/apply`, null)
