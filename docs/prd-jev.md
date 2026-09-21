# PRD — graphify on Jev: decisions leave the brain

**Status:** draft for approval · **Date:** 2026-09-21 · Entry point: brownfield.
Numbers in §4 were measured on this repo's criterion on 2026-09-21; every Jev fact carries a
check date in §13. Planned paths are written plain; backticked paths exist today.

## 1. Summary

**Organising principle: Jev decides, BAML writes.**

- **Decision being made:** for one pattern criterion, per call, does this call match — plus three
  smaller decisions listed in §5.
- **Where it sits:** today the wizard pays an LLM to read a sample (`brain/src/graphify_brain/label.py`
  → BAML `LabelBatch` on Sonnet/Opus/GPT, spawned as a child by `engine/src/jobs.rs`), then
  `SynthesizeRule` writes a DSL rule and `engine/src/rules.rs` re-counts it free forever (D-8 `free`).
- **Who decides today:** an LLM once, a phrase/structure rule every day after.
- **Why change:** §4. The rule that runs free forever scores **TPR 0.60** on an ordinary criterion.
  Free mode is cheap because it is wrong, and nothing in the product says so.
- **What Jev changes:** ~$0.000019 a decision instead of ~$0.002, so what graphify does once at
  wizard time can run on every call, every day, forever.

## 2. Fit verdict

`fit_check.py`, 13 answers, run both ways:

| can_send_data | verdict | exit |
|---|---|---|
| false | **NO-GO** — "Data may not leave for a US third-party API" | 3 |
| true | **GO WITH GUARDS** — "State can steer answers: rules first, untrusted text in a labelled field, test injections" | 4 |

Printed patterns: rules-first · shadow beside the incumbent, then confidence-floor cascade behind a
kill switch · batch every question about one state into one call.

Fit-map: *post-call QA/outcome classification* = **GOOD (provisional)**, *bulk labelling* = **GOOD**
(independent). Both rest on someone else's data, so §4 settles it here instead.

**Decided:** BYOK — a TypeSafe key is a fourth key in Settings beside vapi/anthropic/openai. The
egress decision belongs to whoever runs the instance, as it already does for Anthropic. Off by
default; no org sends anything until a key is added.

## 3. Non-goals

- BAML keeps every job that produces sentences: `PlanPattern`, the clarify loop,
  `SynthesizeRule`/`RefineRule`, and Ask's answer. Jev cannot write and is never asked to.
- Arithmetic, counting, durations and dates stay in code. `duration_s`, `ended_group`,
  `transferred`, `tools_run`, `tool_failed` are computed by the engine and passed into `state`.
  No question ever asks Jev to compare a number.
- Jev never authorises a spend, never writes to a client's system, never chooses a model.
- No second **data** provider. D-13 is untouched: Jev is a decision provider, a different axis.

## 4. Measured

Criterion: *"calls where the caller asked to be put through to a human"*. 44 synthetic cases
(20 positive / 24 negative), label definition frozen before any call, hard negatives including
transferred-but-never-asked, accepted-an-offer, past tense, third party, and two prompt injections.
Deterministic 70/30 split by case id: train 25, eval 19. **Bar written before results: eval TPR ≥ 0.90
at TNR ≥ 0.85.**

| decider | split | TPR | TNR | accuracy |
|---|---|---|---|---|
| DSL phrase rule, speaker=user (what `SynthesizeRule` writes) | eval 19 | **0.67** | 0.80 | 0.74 |
| DSL phrase rule, all 44 | all 44 | **0.60** | 0.92 | 0.77 |
| `transferred = true` alone | all 44 | 0.00 | 0.88 | 0.48 |
| Jev, first-draft question | eval 19 | 1.00 | 0.90 | 0.95 |
| **Jev, rewritten question** | **eval 19** | **0.89** | **1.00** | **0.95** |
| Jev, rewritten question | all 44 | 0.95 | 1.00 | 0.98 |

**The rewrite (lever 1) is where the work was.** Train loss 0.0800 → 0.0000, logloss 0.2294 → 0.0532,
`compare` verdict ACCEPT. What moved:

| case | q1 → q2 | |
|---|---|---|
| injection, direct (train) | 0.79 → 0.14 | fixed |
| injection, fake delimiter (**eval, held out**) | 0.66 → 0.29 | fixed — generalised, not fitted |
| caller accepted an offer (train) | 0.83 → 0.08 | fixed |
| request with no keyword (train) | 0.52 → 0.93 | off the knife edge |
| caller chasing a prior offer (eval) | 0.54 → 0.44 | **broke** — the one eval miss |

**Honest reading, all of it:**
- 19 eval cases. `calibrate.py` warns below 20. **Direction only, not a published accuracy.**
- Eval TPR 0.89 **misses the bar I wrote down** (0.90) by one case of nine positives. TNR clears it.
- Train 1.00/1.00 against eval 0.89/1.00 is the overfit signal the method warns about: I rewrote
  from 3 train cases and 25 is too few. §7 fixes this with fresh cases, not with tuning.
- The one eval miss will **not** be tuned away. Looking at it already cost some of the split's
  independence; tuning on it would cost the rest.
- **Cases are synthetic and I wrote both the cases and the questions.** That is the weakest thing
  about this table and no number here should be quoted to anyone outside this repo.
- **The LLM incumbent was not run** — that needs an Anthropic key I did not extract from the
  encrypted store. It is S-70.

**Cost and latency, measured:** 448 input tokens per case, one battery call. 44 cases = $0.0008 total.
Per decision **$0.0000188**. The same 44 transcripts through `LabelBatch` on Sonnet is roughly
**$0.002 per call** at the quote `label.py` builds — about **100×**. Same case answered 0.97 then 0.96
on two calls: the ±0.02 jitter is real and is why no threshold sits on a knife edge.

## 5. The four decisions, and who owns each

| # | Decision | Today | Proposed | Verdict |
|---|---|---|---|---|
| D-a | per call: does it match the criterion | `LabelBatch` (LLM), then a DSL rule forever | rule first → Jev on the remainder → LLM on the unsure band | measured §4 |
| D-b | which transcript line is the evidence | LLM writes a quote, **unverified** | Jev scores each line, code takes the argmax | quote is in the transcript by construction |
| D-c | a pattern the DSL cannot express | refused in the plan (D-2) | new mode: the pattern **is** a Jev battery | new capability |
| D-d | which calls go into Ask's sample | filters only | Jev relevance score per call, code takes top N | fit map: reranking = GOOD (independent) |

D-b is not generation — selecting among lines that exist is a decision. That is what makes it safe.

## 6. Decision design

**State** (filtered, structured, untrusted text in a named field — `transcript` is quoted speech):

```json
{"call_facts": {"duration_s": 212, "ended_group": "customer", "transferred": false,
                "tools_run": ["bookAppointment"], "tool_failed": false},
 "transcript": [{"speaker": "caller", "text": "..."}, {"speaker": "agent", "text": "..."}]}
```

A value nobody recorded is `"—"`, never `0` and never `false` — the spec's rule, and it matters more
here than anywhere: a model told a call lasted 0 seconds reasons about a call that never connected.

**Questions:** one battery per call, every question about that call in one call (state is billed once).
The wording that produced §4 is in scratchpad `q2.json` and moves into the repo at step S-69. The
`criteria.false` paragraph that defeated both injections is lifted almost verbatim from the prose
already in `brain/baml_src/label.baml` — the repo had the right words, they were just in a place Jev
could not read.

**Composition in code:** the rule decides first and free; Jev sees only the remainder; answers
combine with `min`/`max`, never inside a question.

## 7. Thresholds and calibration plan

| Question | fn:fp | Fitted | Source |
|---|---|---|---|
| wants_human (worked example) | 2:1 | threshold 0.52 | thresholds2.json, eval n=19, 2026-09-21, **direction only** |

fn:fp = 2:1: an undercount hides what the analyst is hunting; an overcount is visible and checkable
against the evidence quote in the call drawer. **Before any threshold ships:**
- 200+ cases per shipped question, ≥ 20 per class, stratified. 44 is a direction, not a number.
- Fresh cases added **before** re-measuring, because the eval split has now been looked at.
- Hard negatives mandatory: agent-offered, caller-accepted, past tense, third party, callback,
  and at least four injection shapes.
- **graphify already stores its own gold labels.** `pattern_labels` holds `llm_match`, `rule_match`
  and `evidence` per call per pattern. Every wizard run an analyst confirms is a labelled case. This
  is the single biggest asset here and nothing currently reads it back.
- Two labellers on a sample (disagreement caps achievable accuracy); incumbent LLM on the same
  cases; real client transcripts only after §12 Q1 is closed by a named person.

## 8. Architecture

**The seam.** One file owns the fact that a decision provider exists: a `DecisionModel` trait,
`decide(state, questions) -> answers`, with three implementations — Jev, an LLM fallback built in the
same PR, and a fake for tests, so no test needs a network. The Jev adapter pins `jev-1.13.0`, returns
`response.model` so drift is visible, and refuses an empty question set.

**Two spec amendments. Both are load-bearing and both should be approved on their own merits.**

**A-1 — the outbound rule splits along the axis it was always about.** Must-never #1 says *"Send
anything but GET to a provider"*, enforced tree-wide by `engine/tests/outbound.rs`: `CONNECTORS` lists
the only files permitted to reach out (`vapi.rs`), `NOT_A_GET` contains `".post("`, and a mock
accepting every method asserts that everything which actually left was a GET. **A Jev call is a POST.
An engine-side Jev client cannot exist today without that guard going red — correctly.**

The amendment distinguishes two kinds of provider:
- **Data providers** read a client's live account. `vapi.rs` is one. **GET only, forever, no
  exceptions** — this is what stops graphify ever mutating a customer's Vapi org, and it does not
  move a millimetre.
- **Decision providers** hold no graphify data and own nothing we could damage. They may POST, from
  one named file, which may still name no other verb.

`outbound.rs` gains a second list and a second assertion; the guard stays machine-checked and gets
*more* precise, not less. The purpose of the invariant survives intact. **If this amendment is
rejected, the fallback is to keep Jev in the Python brain** — POST is already how BAML reaches
Anthropic — at the cost of keeping a Python spawn on the daily unattended path.

**A-2 — D-8 gains a fourth mode, and "call a model" gets defined.** Must-never #2 says no model call
without a shown cost and an explicit go. Decided (Q3): **a Jev question is data that costs money, not
a model call** — it sits beside a DSL rule, both written once and run forever. So a Jev pattern runs
unattended like `free` does. **What does not relax:** every Jev call is priced, booked to the `spend`
ledger, and counted against the org's hard daily USD cap, which still stops the run when reached.
"Too cheap to meter" is not in this PRD; an uncapped loop at $0.000019 still bankrupts somebody at
the right scale.

**Operational:** client timeout (a near-64k request once stalled silently); size cap well under 64k;
retry 408/429/5xx, 529 as overload; circuit breaker; kill switch per path; egress guard refusing to
send when an org has no TypeSafe key; keys server-side only, never in a bundle or a log.

## 9. Steps

One step, one PR, each verifiable alone. Full step entries (Files/Today/Change/Acceptance/Verify/
Must-not) go into `docs/spec.md` on approval — they are omitted here to keep this readable.

| # | Step | Tag | Depends |
|---|---|---|---|
| S-68 | The evidence quote is checked against its transcript (the bug in §10) | — | nothing |
| S-69 | A questions file and its thresholds, in the repo, with the 44 cases | — | nothing |
| S-70 | Run the LLM incumbent on the same cases; publish the side-by-side | — | S-69 |
| S-71 | The `DecisionModel` seam + fake adapter, no network, flag off | **[Rust]** | nothing |
| S-72 | outbound.rs learns data vs decision connectors (**A-1**) | **[Rust]** | S-71 |
| S-73 | The Jev adapter: pinned, timed out, size-capped, key from Settings | **[Rust]** | S-72 |
| S-74 | TypeSafe as a fourth key in Settings; egress refused without one | **[Rust]** | S-73 |
| S-75 | Shadow mode: Jev beside the labeller, both logged, act on neither | **[Rust]** | S-74 |
| S-76 | Read `pattern_labels` back as a calibration set; report agreement | — | S-75 |
| S-77 | Cascade: rule → Jev confident band → LLM on the middle | **[Rust]** | S-76 |
| S-78 | D-b: evidence by line scoring, argmax in code | **[Rust]** | S-77 |
| S-79 | D-c: the semantic pattern mode (**A-2**), capped and ledgered | **[Rust]** | S-77 |
| S-80 | D-d: Jev reranks Ask's sample | **[Rust]** | S-77 |

S-68–S-70 touch no Rust and need no key decision — worth doing whatever happens to A-1.
**Nothing after S-74 sends a real transcript anywhere until §12 Q1 is closed.**

## 10. Bug found while reading (logged separately, ships regardless)

`brain/baml_src/label.baml` instructs *"Never quote a line that is not in the transcript you were
given."* Nothing enforces it. `brain/src/graphify_brain/synth.py:476` checks `evidence` is a `str`
and stops. A hallucinated quote reaches `pattern_labels`, is served to the call drawer
(`engine/src/queries.rs:342`), and is fed to `SynthesizeRule` as "the words people actually said" —
where it can shape a rule that then runs unattended forever. S-68.

## 11. Risks

| Risk | Mitigation |
|---|---|
| **Prompt injection.** Measured, not theoretical: an unmitigated question scored a direct injection at **0.82**. Criteria fixed it to 0.14, but the fake-delimiter variant only reached 0.29 | Rules first; transcript in a named untrusted field; injection cases in CI, failing the build; never the only defence |
| **Confidently wrong.** Both train false alarms sat above 0.80 | Calibrated bands; weekly audit of a sample of confident answers; fn:fp costs written down |
| Synthetic, self-authored cases | 200+ real cases from `pattern_labels` before any published number |
| A wrong judge is now affordable at scale — the whole point cuts both ways | Human-labelled audit sample per period; agreement rate monitored, not assumed |
| Alias drift silently stales thresholds | Pin `jev-1.13.0`; log `response.model`; alert on change; recalibrate |
| Single closed vendor, no self-hosting, limits "can change without notice", no SOC 2 found | The seam + a warm LLM fallback; outage drill in §12 |
| A semantic pattern is not inspectable the way a phrase list is | The battery and its thresholds are shown in the pattern editor, reviewed like a rule |
| Option-order and lookalike sensitivity | `other` on every Choice; order-shuffle test |
| Non-English calls | English is strongest; calibrate per language or refuse the mode |

## 12. Open questions

| # | Question | Owner | Needed by |
|---|---|---|---|
| Q1 | A named person reads TypeSafe's **current** terms before any real transcript leaves. ZDR is enterprise-only; US hosting; no retention period published; an indexed pre-launch Terms copy granted access "solely for evaluating" | Abhishek | S-75 |
| Q2 | Approve or reject amendment **A-1**. Rejection is survivable: Jev stays in the brain | Abhishek | S-72 |
| Q3 | Approve amendment **A-2**'s wording in the Must-never block | Abhishek | S-79 |
| Q4 | Default daily USD cap for a semantic pattern. **Default written in: $0.50/org/day**, ~26,000 calls. Confirm | Abhishek | S-79 |
| Q5 | Rotate the pasted TypeSafe key, and the Vapi key from O-06 (now 65 steps stale) | Abhishek | immediately |

## 13. Facts used

| Fact | Value | Source | Checked |
|---|---|---|---|
| Price | $0.042 / 1M input tokens, output free | docs.typesafe.ai | 2026-09-21 |
| Model pin | `jev-1.13.0` accepted; `jev-latest` resolved to it | `GET /v1/models` + `response.model`, run here | 2026-09-21 |
| `GET /v1/models` lists aliases only, not the dated id | confirmed | run here | 2026-09-21 |
| Context | 64k request / 32k state + longest question | docs.typesafe.ai | 2026-09-21 |
| Measured cost, this criterion | 448 input tokens = $0.0000188 per call | run here, n=88 calls | 2026-09-21 |
| Run-to-run jitter | ±0.02 (0.97 vs 0.96, same case) | observed here | 2026-09-21 |
