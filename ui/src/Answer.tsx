// The answer, drawn.
//
// The brain returns Markdown, and this draws the small part of it the prompt in
// `ask.baml` promises to write: `## ` headings, paragraphs, `- ` bullets, `**bold**` and
// `` `code` ``. No parser library, and no `dangerouslySetInnerHTML` — every piece below
// becomes a React element, so a model that emitted a `<script>` has emitted five
// characters that get drawn as five characters.
//
// The subset is small on purpose and the prompt names the same one. Anything outside it
// is rendered literally, which is the honest failure: the reader sees exactly what the
// model wrote rather than a silently dropped table.

import type { ReactNode } from 'react'

/** Split on the two inline markers, keeping them. A capturing group is what makes `split`
 * return the delimiters, so one pass gives text and markup in order. */
const INLINE = /(\*\*[^*\n]+\*\*|`[^`\n]+`)/g

function inline(text: string): ReactNode[] {
  return text
    .split(INLINE)
    .filter((part) => part !== '')
    .map((part, i) => {
      if (part.startsWith('**') && part.endsWith('**')) return <strong key={i}>{part.slice(2, -2)}</strong>
      if (part.startsWith('`') && part.endsWith('`')) return <code key={i}>{part.slice(1, -1)}</code>
      return part
    })
}

/** One block of the answer: a heading, a list, or a paragraph. */
type Block =
  | { kind: 'h'; text: string }
  | { kind: 'ul'; items: string[] }
  | { kind: 'p'; text: string }

/** Group the lines into blocks.
 *
 * A blank line closes whatever was open, which is the whole of the rule: consecutive
 * `- ` lines are one list, consecutive plain lines are one paragraph, and a heading is
 * always its own block.
 */
function blocks(markdown: string): Block[] {
  const out: Block[] = []
  let open: Block | null = null
  for (const raw of markdown.split('\n')) {
    const line = raw.trim()
    if (line === '') {
      open = null
      continue
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(line)
    if (heading) {
      out.push({ kind: 'h', text: heading[1] })
      open = null
      continue
    }
    const bullet = /^[-*]\s+(.*)$/.exec(line)
    if (bullet) {
      if (open?.kind === 'ul') open.items.push(bullet[1])
      else out.push((open = { kind: 'ul', items: [bullet[1]] }))
      continue
    }
    // A wrapped sentence joins the paragraph above it rather than starting a new one:
    // Markdown's own rule, and the reason a hard-wrapped answer does not come out as one
    // paragraph per line.
    if (open?.kind === 'p') open.text += ` ${line}`
    else out.push((open = { kind: 'p', text: line }))
  }
  return out
}

export default function Answer({ markdown }: { markdown: string }) {
  return (
    <div className="answer">
      {blocks(markdown).map((block, i) => {
        if (block.kind === 'h') return <h3 key={i}>{inline(block.text)}</h3>
        if (block.kind === 'ul')
          return (
            <ul key={i}>
              {block.items.map((item, j) => (
                <li key={j}>{inline(item)}</li>
              ))}
            </ul>
          )
        return <p key={i}>{inline(block.text)}</p>
      })}
    </div>
  )
}
