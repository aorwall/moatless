/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Backend API origin for dev; empty/unset in production (relative URLs). */
  readonly VITE_API_URL?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
