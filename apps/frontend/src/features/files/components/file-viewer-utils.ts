export function isMarkdownFile(filePath: string): boolean {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  return ext === 'md' || ext === 'mdx'
}

export function isHtmlFile(filePath: string): boolean {
  const ext = filePath.split('.').pop()?.toLowerCase() || ''
  return ext === 'html' || ext === 'htm'
}

export function isPreviewable(filePath: string): boolean {
  return isMarkdownFile(filePath) || isHtmlFile(filePath)
}
