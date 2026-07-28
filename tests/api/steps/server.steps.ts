/**
 * Server steps.
 *
 * Waiting for a preview server is `waitForPreviewReady` in the shared wait
 * helpers; what a chain cannot do with `api` alone is reach the served URL.
 */

/** Fetch a URL a preview server is serving, without hanging the test on it. */
export async function fetchServerUrl(
  url: string,
  timeoutMs = 10_000,
): Promise<{ status: number; body: string }> {
  const response = await fetch(url, {
    signal: AbortSignal.timeout(timeoutMs),
  })
  const body = await response.text()
  return { status: response.status, body }
}
