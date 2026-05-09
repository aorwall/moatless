import { customInstance } from '@moatless/api-client/custom-instance'

export interface TaskTemplateResponse {
  id: string
  name: string
  description?: string | null
  icon?: string | null
  color?: string | null
  repositoryId: string
  agentType: string
  agentMode?: string | null
  defaultModel?: string | null
  initialPrompt?: string | null
  skills?: string[] | null
  scope: 'user' | 'team' | 'global'
  teamId?: string | null
  featured: boolean
  displayOrder: number
  createdBy?: string | null
  createdAt: string
  updatedAt: string
}

export const taskTemplatesApi = {
  listTaskTemplates(): Promise<TaskTemplateResponse[]> {
    return customInstance<TaskTemplateResponse[]>('/api/v1/task-templates')
  },

  getFeaturedTaskTemplates(limit = 6): Promise<TaskTemplateResponse[]> {
    return customInstance<TaskTemplateResponse[]>(`/api/v1/task-templates/featured?limit=${limit}`)
  },

  getTaskTemplate(id: string): Promise<TaskTemplateResponse> {
    return customInstance<TaskTemplateResponse>(`/api/v1/task-templates/${id}`)
  },
}
