/**
 * The one thing the annotation runtime needs from its host: a way to send a
 * named message and a way to hear one back.
 *
 * Everything else the runtime does — hit-testing, drawing, capturing element
 * context — is plain DOM work that runs the same wherever the page is shown.
 * Keeping the pipe behind this interface is what lets one implementation serve
 * both an Electron `<webview>` preload and a script loaded by a framed page.
 */

export type AnnotationListener = (...args: readonly unknown[]) => void

export interface AnnotationTransport {
  send(channel: string, ...args: readonly unknown[]): void
  on(channel: string, listener: AnnotationListener): void
  off(channel: string, listener: AnnotationListener): void
}
