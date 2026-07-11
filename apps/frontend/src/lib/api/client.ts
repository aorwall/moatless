import type { ApiErrorResponse } from '@/lib/types/api-errors'

// In production, use relative URLs since frontend is served by backend
// In development, use the configured API URL or default to localhost
const API_BASE_URL = import.meta.env.PROD
  ? '' // Empty string for relative URLs in production
  : (import.meta.env.VITE_API_URL ?? '')

export class ApiError extends Error {
  status: number
  statusText: string
  data?: ApiErrorResponse | unknown

  constructor(status: number, statusText: string, data?: ApiErrorResponse | unknown) {
    super(`API request failed: ${status} ${statusText}`)
    this.name = 'ApiError'
    this.status = status
    this.statusText = statusText
    this.data = data
  }

  get isApiError(): boolean {
    return (
      typeof this.data === 'object' &&
      this.data !== null &&
      'error' in this.data &&
      typeof (this.data as { error: unknown }).error === 'object'
    )
  }

  get apiError(): ApiErrorResponse | undefined {
    return this.isApiError ? (this.data as ApiErrorResponse) : undefined
  }
}

interface RequestOptions extends RequestInit {
  params?: Record<string, string | number | boolean | undefined> | undefined
}

// Shared request pipeline: build the URL, send the fetch, and surface non-2xx
// responses as `ApiError`. Returns the raw `Response` so the typed wrappers below
// decide how to decode the body.
async function performRequest(path: string, options: RequestOptions = {}): Promise<Response> {
  const { params, ...fetchOptions } = options

  let url = `${API_BASE_URL}${path}`

  if (params) {
    const searchParams = new URLSearchParams()
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined) {
        searchParams.append(key, String(value))
      }
    }
    const queryString = searchParams.toString()
    if (queryString) {
      url += `?${queryString}`
    }
  }

  // Only set Content-Type when there's a body
  const headers: Record<string, string> = {
    ...(fetchOptions.headers as Record<string, string>),
  }
  if (fetchOptions.body !== undefined) {
    headers['Content-Type'] = 'application/json'
  }

  const response = await fetch(url, {
    ...fetchOptions,
    headers,
    credentials: 'include', // Include cookies for session management
  })

  if (!response.ok) {
    let errorData: unknown
    try {
      errorData = await response.json()
    } catch {
      errorData = null
    }

    throw new ApiError(response.status, response.statusText, errorData)
  }

  return response
}

async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const response = await performRequest(path, options)

  // A 204 has no body to decode. Callers that expect an empty response should
  // use `requestVoid`/the `*Void` methods so a `null` never masquerades as a
  // non-nullable `T`. If a caller typed a non-void `T` still receives a 204,
  // this is the honest failure point rather than a silent `null`.
  if (response.status === 204) {
    return null as unknown as T
  }

  // THE APP'S ONLY SANCTIONED NETWORK-BOUNDARY CAST.
  // `request<T>` cannot statically verify that the decoded JSON matches `T`;
  // `T` is guaranteed instead by the generated OpenAPI types the callers pass.
  // Do not add response-shape casts elsewhere — for high-risk endpoints, layer
  // a Zod `parse()` in the feature API module instead.
  return (await response.json()) as unknown as T
}

// Void variant: decodes nothing and returns `void`, so an empty/204 response can
// never leak a `null` into a typed value. Prefer this for POST/PUT/PATCH/DELETE
// endpoints that return no meaningful body.
async function requestVoid(path: string, options: RequestOptions = {}): Promise<void> {
  await performRequest(path, options)
}

export const apiClient = {
  get<T>(path: string, params?: Record<string, string | number | boolean | undefined>): Promise<T> {
    return request<T>(path, { method: 'GET', params })
  },

  post<T>(path: string, data?: unknown): Promise<T> {
    return request<T>(path, {
      method: 'POST',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  put<T>(path: string, data?: unknown): Promise<T> {
    return request<T>(path, {
      method: 'PUT',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  patch<T>(path: string, data?: unknown): Promise<T> {
    return request<T>(path, {
      method: 'PATCH',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  delete<T>(path: string): Promise<T> {
    return request<T>(path, { method: 'DELETE' })
  },

  // Void-returning variants for endpoints with no response body (see `requestVoid`).
  postVoid(path: string, data?: unknown): Promise<void> {
    return requestVoid(path, {
      method: 'POST',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  putVoid(path: string, data?: unknown): Promise<void> {
    return requestVoid(path, {
      method: 'PUT',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  patchVoid(path: string, data?: unknown): Promise<void> {
    return requestVoid(path, {
      method: 'PATCH',
      ...(data !== undefined && { body: JSON.stringify(data) }),
    })
  },

  deleteVoid(path: string): Promise<void> {
    return requestVoid(path, { method: 'DELETE' })
  },
}
