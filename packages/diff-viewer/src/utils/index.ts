export { getFileStatus, type ParseDiffOptions, parseDiff } from './diff-parser.js'
export { type HighlightToken, highlightLine, isLanguageSupported } from './syntax-highlight.js'
export {
  computeCharDiff,
  computeWordDiff,
  hasWordChanges,
  tokenize,
} from './word-diff.js'
