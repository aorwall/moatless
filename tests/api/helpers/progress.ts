/**
 * What a test says while it is running.
 *
 * A run against the cluster takes minutes, and three chains interleave their
 * output, so silence is indistinguishable from a hang and an untagged line is
 * indistinguishable from another chain's. Every line goes out labelled with the
 * test that emitted it, which the chain does not have to repeat and cannot get
 * wrong.
 *
 *     progress(`sandbox ready, ${files.length} files present`);
 *     → [SANDBOX-03.2: Start stopped sandbox] sandbox ready, 42 files present
 *
 * This is narration, not evidence. What a test proves belongs in an assertion
 * message, which is what a failure actually prints.
 */

import { test } from '@playwright/test'

export function progress(message: string): void {
  console.log(`[${currentTest()}] ${message}`)
}

/** The running test, or the file's name when called outside one. */
function currentTest(): string {
  try {
    const info = test.info()
    return info.title || info.titlePath.join(' › ')
  } catch {
    return 'chain'
  }
}
