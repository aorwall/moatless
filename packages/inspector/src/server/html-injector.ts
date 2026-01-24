/**
 * Injects a script tag into HTML content.
 * Tries to inject before </body>, then </html>, or appends to end.
 */
export function injectScriptIntoHtml(html: string, scriptContent: string): string {
  const scriptTag = `<script>${scriptContent}</script>`
  const injectionPoint = '</body>'

  if (html.includes(injectionPoint)) {
    return html.replace(injectionPoint, `${scriptTag}${injectionPoint}`)
  }
  if (html.includes('</html>')) {
    return html.replace('</html>', `${scriptTag}</html>`)
  }
  // Fallback: append to end
  return html + scriptTag
}
