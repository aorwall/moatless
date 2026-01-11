// Routes
export { registerFileRoutes } from './api/files.routes'
export type { FileSearchRequest, FileSearchResult } from './services/file.service'
// Services
export { FileService } from './services/file.service'

// Utils
export { buildFileTree } from './utils/file-tree-builder'
