/**
 * A live event stream that lasts as long as a chain does.
 *
 * Playwright's fixtures are scoped to a test or to a worker; a serial chain
 * needs the scope in between — one stream opened before its first test and
 * closed after its last, so that a test can ask what happened during the ones
 * before it. `chainEvents()` registers those two hooks and hands back an
 * accessor:
 *
 *     const events = chainEvents();
 *     ...
 *     const started = await events().waitForEvent({ type: "agent", ... });
 *
 * Call it at describe scope, before the chain's own `beforeAll`, so the stream
 * is already open when the chain creates the task it wants events for.
 */

import { EventStream } from '../../shared/event-stream'
import { test } from '../fixtures/api-fixtures'

export function chainEvents(): () => EventStream {
  let stream: EventStream | null = null

  test.beforeAll(async ({ driver }) => {
    stream = new EventStream(driver)
    await stream.start()
  })

  test.afterAll(() => {
    stream?.stop()
    stream = null
  })

  return () => {
    if (!stream) {
      throw new Error(
        "the chain's event stream is not open — chainEvents() must be called at describe scope",
      )
    }
    return stream
  }
}
