export interface RepositoryTemplate {
  id: string
  name: string
  description?: string
  gitUrl: string
  setupCommand?: string
  previewConfig?: {
    serverStartCommands?: string[]
    serverPort?: number
  }
}

export interface TemplateListResponse {
  templates: RepositoryTemplate[]
}
