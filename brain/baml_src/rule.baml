// Turning what a model saw into something the engine can run without it.
//
// This is the point of the whole wizard. A model read the calls once and said which ones
// matched; what comes out of here is a rule in the DSL that reproduces those judgements
// on its own, so every call after this one is classified for nothing. A pattern that runs
// in `free` mode is a pattern whose model has already been paid for and dismissed.
//
// Nothing here is executed. The rule is data — `engine/src/rules.rs` is the only thing
// that reads one, and `graphify rule-check` is the only thing that runs one. The classes
// below exist so that a model cannot return a key that DSL does not have.
//
// Both run on the model the analyst picked for the labelling. Writing the rule is the
// harder half of the job — the labels are the evidence and this is the generalisation —
// so it is not the place to save a few cents on a smaller model.

// The DSL, field for field as `engine/src/rules.rs` spells it. Every list means "any of
// these", so `tool_not_called` is the plain negation of `tool_called`. The lists are not
// optional: a model that has to write `[]` has to decide the condition does not apply,
// where one that may omit the key can leave it out by accident.
class Rule {
  any_phrases string[] @description("Case-insensitive substrings. A line containing any one of them hits.")
  regex string[] @description("Regular expressions over the same lines, compiled case-insensitively.")
  speaker string? @description("user, bot, or any. Null is any. Whose lines the phrases and regexes may look at.")
  ended_reasons string[] @description("Raw Vapi ended reasons, any one of which counts.")
  ended_groups string[] @description("Ended groups, any one of which counts.")
  tool_called string[] @description("Tool names, any one of which having run counts.")
  tool_not_called string[] @description("Tool names, none of which may have run.")
  tool_failed bool? @description("True: some tool call failed. False: none did. Null: do not ask.")
  transferred bool? @description("True: the call was handed to a person. False: it was not. Null: do not ask.")
  min_duration_s float? @description("The call lasted at least this long.")
  max_duration_s float? @description("The call lasted at most this long.")
}

enum ChartKind {
  Line @description("The count per time bucket, drawn as a trend. The right answer for almost every pattern.")
  Bar @description("The count per time bucket, drawn as a tally. For a pattern that is about volume in a period rather than a direction.")
}

class Chart {
  kind ChartKind
  title string @description("What the chart is counting, in the analyst's own words. Four or five words.")
}

class Synthesis {
  rule Rule
  chart Chart
  reason string @description("One or two sentences: what the rule keys on, and what it will miss.")
}

// `RefineRule` returns a reason of its own, and not just the rule. The synthesis reason
// describes a rule that a refinement has replaced; printing it beside the new rule would
// be an explanation of something nobody is running. The chart is not re-suggested — what
// a pattern counts does not change because its rule got sharper.
class Refinement {
  rule Rule
  reason string @description("One or two sentences: what you changed, and what the rule will still miss.")
}

// One call the rule and the labeller disagree about. No call id: the model cannot look a
// call up, and an id it can neither read nor check is a UUID paid for by the token.
class Disagreement {
  labelled bool @description("What the labeller decided after reading the whole transcript.")
  matched bool @description("What your rule decided. These two differ; that is why this call is here.")
  evidence string @description("The line the labeller quoted as deciding it.")
}

function SynthesizeRule(criterion: string, plan: Plan, labels: LabelForRule[], dsl: string) -> Synthesis {
  client Sonnet
  prompt #"
    {{ _.role("system") }}
    You write the rule that replaces you.

    A model has already read these calls and decided which ones match. Your job is to write
    a rule in the DSL below that reaches the same verdicts on its own, so that every call
    after today is classified without a model and without a bill.

    Work from the evidence, not from the criterion. The criterion and the plan say what was
    meant; the quotes below are what people actually said, and a rule is only ever as good
    as the words it keys on. Prefer the phrasing that appears in the matching calls and
    nowhere in the others.

    Write the smallest rule that separates them.

    - A phrase that appears in a call that did not match is a phrase that will cost you.
    - Structural conditions — an ended group, a tool, a transfer, a duration — separate
      calls that no phrase can. Reach for them when the evidence points at them, and not
      because they are available.
    - `any_phrases` before `regex`. A regex is for a shape a list of phrases cannot state,
      and every one of them is a thing the next person has to read.
    - A list you do not want is `[]`, and a condition you do not want to ask about is null.
      An empty rule matches every call.

    You will be wrong about some of them, and that is measured: the rule is run over these
    same calls and the fraction it agrees with is reported. Say in `reason` what you expect
    it to miss.

    ---
    {{ dsl }}
    ---

    {{ ctx.output_format }}

    {{ _.role("user") }}
    The criterion: {{ criterion }}

    The plan the analyst confirmed:
    {{ plan }}

    What the labeller decided, one call per line:
    {% for l in labels %}
    {{ "MATCH    " if l.match else "no match " }} {{ l.evidence }}
    {% endfor %}
  "#
}

function RefineRule(criterion: string, plan: Plan, rule: Rule, disagreements: Disagreement[], dsl: string) -> Refinement {
  client Sonnet
  prompt #"
    {{ _.role("system") }}
    Your rule has been run over the calls it was written from, and these are the ones it
    got wrong.

    Return one rule: the whole thing, not a patch.

    A call the labeller matched and your rule missed wants something added — usually a
    phrase you did not think of, sometimes a condition that was too narrow. A call your
    rule matched and the labeller did not wants something taken away or made stricter.

    Do not simply widen it. Every phrase you add to catch one call is a phrase that will
    catch calls you have not seen, and this rule will run unattended on every call from now
    on. A rule that agrees on fewer of these and is right about the ones it is sure of beats
    one that agrees on all of them by naming each of them.

    Some of these will be the labeller's mistakes, not yours. A disagreement whose evidence
    does not support the label it was given is one to leave alone.

    Say in `reason` what you changed and what the rule will still miss. It is printed
    beside the rule, in place of the one the first draft came with.

    ---
    {{ dsl }}
    ---

    {{ ctx.output_format }}

    {{ _.role("user") }}
    The criterion: {{ criterion }}

    The plan the analyst confirmed:
    {{ plan }}

    The rule you wrote:
    {{ rule }}

    Where it disagrees with the labeller:
    {% for d in disagreements %}
    labelled {{ "MATCH" if d.labelled else "no match" }}, rule said {{ "MATCH" if d.matched else "no match" }} — {{ d.evidence }}
    {% endfor %}
  "#
}

// The labeller's verdict as the rule-writer needs it: what it decided and why. The call id
// is left out for the same reason it is left out of `Disagreement`.
class LabelForRule {
  match bool
  evidence string
}
