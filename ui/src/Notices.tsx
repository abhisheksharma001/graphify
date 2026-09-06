// What went wrong where nobody was looking.
//
// Two failures in the engine used to reach stderr and stop there: the boot sweep that
// clears jobs a dead process left live, and the close that ends a job. Both are the
// database refusing a write to `jobs`, both cost a live slot until the next restart, and
// neither was visible to the one person who needed to know — whoever has this tab open.
//
// The banner is above every screen because an outage is not a property of the screen you
// happen to be on, and it has no dismiss button because the condition is true until it is
// not. What ends it is a restart, which re-runs the sweep and either clears the rows or
// says so again.

import { useEffect, useState } from 'react'
import * as api from './api'
import type { Notice } from './api'

/** Slow on purpose. A close that fails while somebody is watching has to arrive without a
 * reload, and nothing here changes by the second: what it reports lasts until a restart. */
export const POLL_MS = 30_000

export default function Notices() {
  const [notices, setNotices] = useState<Notice[]>([])
  const [dropped, setDropped] = useState(0)

  useEffect(() => {
    let live = true
    const ask = () =>
      api
        .notices()
        .then((seen) => {
          if (!live) return
          setNotices(seen.notices)
          setDropped(seen.dropped)
        })
        // Deliberately silent. A request nobody made failing is not news, and putting it
        // on the error line would mean the notices banner's own outage shouting over the
        // outage it exists to report. What was on screen stays on screen.
        .catch(() => {})
    ask()
    const timer = setInterval(ask, POLL_MS)
    return () => {
      live = false
      clearInterval(timer)
    }
  }, [])

  if (notices.length === 0) return null

  return (
    // `role="status"`, not `alert`: a reader arriving mid-session should be told, but this
    // is not something that appeared because of anything they just did.
    <section className="notices" role="status">
      <h2>graphify could not write to its database</h2>
      <ul>
        {notices.map((n) => (
          <li key={`${n.at}${n.text}`}>
            <time dateTime={n.at}>{n.at.slice(11, 19)}</time> {n.text}
          </li>
        ))}
      </ul>
      {dropped > 0 && (
        <p className="dropped">
          {dropped} older {dropped === 1 ? 'notice' : 'notices'} were not kept.
        </p>
      )}
    </section>
  )
}
