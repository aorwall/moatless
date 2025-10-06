import type { RouteConfig } from '@hono/zod-openapi'
import type { z } from 'zod'

type JsonMediaType = 'application/json'

type ExtractContentSchema<Content> = Content extends { schema: infer Schema } ? Schema : never

type RequestBodyContent<R extends RouteConfig, MediaType extends string> = R['request'] extends {
  body: { content: Record<string, unknown> }
}
  ? MediaType extends keyof R['request']['body']['content']
    ? ExtractContentSchema<R['request']['body']['content'][MediaType]>
    : never
  : never

type ResponseContent<
  R extends RouteConfig,
  Status extends keyof R['responses'],
  MediaType extends string,
> = R['responses'][Status] extends { content: Record<string, unknown> }
  ? MediaType extends keyof R['responses'][Status]['content']
    ? ExtractContentSchema<R['responses'][Status]['content'][MediaType]>
    : never
  : never

type ParamsSchema<R extends RouteConfig> = R['request'] extends {
  params: infer Params
}
  ? Params
  : never

type QuerySchema<R extends RouteConfig> = R['request'] extends {
  query: infer Query
}
  ? Query
  : never

/**
 * Infer the JSON request body type for a route definition.
 */
export type RouteRequestJson<R extends RouteConfig> = RequestBodyContent<
  R,
  JsonMediaType
> extends z.ZodTypeAny
  ? z.input<RequestBodyContent<R, JsonMediaType>>
  : never

/**
 * Infer the validated params type for a route definition.
 */
export type RouteParams<R extends RouteConfig> = ParamsSchema<R> extends z.ZodTypeAny
  ? z.input<ParamsSchema<R>>
  : Record<string, never>

/**
 * Infer the validated query string type for a route definition.
 */
export type RouteQuery<R extends RouteConfig> = QuerySchema<R> extends z.ZodTypeAny
  ? z.input<QuerySchema<R>>
  : Record<string, never>

type ResponseOutputs<R extends RouteConfig> = {
  [Status in keyof R['responses']]: ResponseContent<R, Status, JsonMediaType> extends z.ZodTypeAny
    ? z.output<ResponseContent<R, Status, JsonMediaType>>
    : never
}[keyof R['responses']]

/**
 * Infer the JSON response body type for a route definition.
 */
export type RouteSuccess<R extends RouteConfig> = [ResponseOutputs<R>] extends [never]
  ? unknown
  : ResponseOutputs<R>

/**
 * Build a concrete path by injecting `params` into a route template.
 */
export function buildPath<R extends RouteConfig>(
  route: R,
  params?: Partial<RouteParams<R>>,
): string {
  if (!params) {
    return route.path
  }

  return route.path.replace(/\{([^}]+)\}/g, (_, key: string) => {
    const value = params[key as keyof RouteParams<R>]

    if (value === undefined || value === null) {
      throw new Error(`Missing required path param: ${key}`)
    }

    return encodeURIComponent(String(value))
  })
}
