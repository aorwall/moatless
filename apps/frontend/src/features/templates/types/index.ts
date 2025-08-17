export interface RepositoryTemplate {
  id: string
  name: string
  description?: string
  gitUrl: string
  setupCommand?: string
  previewConfig?: {
    serverStartCommand?: string
    serverPort?: number
  }
}

export interface TemplateListResponse {
  templates: RepositoryTemplate[]
}
