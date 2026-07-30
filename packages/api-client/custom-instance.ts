/**
 * Custom fetch instance for Orval-generated API client.
 *
 * Replicates the behavior of the existing `apiClient` from
 * `apps/frontend/src/lib/api/client.ts` — same-origin requests,
 * credentials included, JSON body serialization.
 *
 * Orval v8 passes standard `RequestInit` fields (`body`, `headers`,
 * `signal`, `method`) directly.  The `body` is already JSON-stringified
 * by the generated code, so we pass it through to `fetch()` as-is.
 */

const baseURL =
	typeof window !== "undefined" && import.meta.env?.PROD
		? ""
		: (import.meta.env?.VITE_API_URL ?? "");

/**
 * The message to put on a thrown request error.
 *
 * `response.statusText` cannot carry this on its own: HTTP/2 dropped the
 * reason phrase, so browsers report it as an empty string against any
 * HTTP/2 deployment. An error thrown with an empty message reaches the UI as
 * a blank alert and a toast with no description — the request failed and the
 * screen says nothing about why. The server's own words are what an operator
 * needs, so they are preferred, in the shapes this API actually returns:
 * `{error: "..."}`, `{error: {message: "..."}}`, `{message}`, `{detail}`.
 */
export function errorMessage(
	status: number,
	statusText: string,
	body: unknown,
): string {
	if (typeof body === "object" && body !== null) {
		const data = body as Record<string, unknown>;
		const nested =
			typeof data["error"] === "object" && data["error"] !== null
				? (data["error"] as Record<string, unknown>)["message"]
				: undefined;
		for (const candidate of [
			data["error"],
			nested,
			data["message"],
			data["detail"],
		]) {
			if (typeof candidate === "string" && candidate.length > 0) {
				return candidate;
			}
		}
	}

	return statusText || `Request failed with status ${status}`;
}

export const customInstance = async <T>(
	url: string,
	options?: RequestInit & { params?: Record<string, string> },
): Promise<T> => {
	const targetUrl = new URL(`${baseURL}${url}`, window.location.origin);

	if (options?.params) {
		for (const [key, value] of Object.entries(options.params)) {
			if (value !== undefined) {
				targetUrl.searchParams.append(key, String(value));
			}
		}
	}

	const response = await fetch(targetUrl.toString(), {
		...options,
		credentials: "include",
	});

	if (!response.ok) {
		const errorData = await response.json().catch(() => null);
		throw Object.assign(
			new Error(errorMessage(response.status, response.statusText, errorData)),
			{
				status: response.status,
				statusText: response.statusText,
				data: errorData,
			},
		);
	}

	if (response.status === 204) {
		return null as T;
	}

	return response.json() as Promise<T>;
};

export default customInstance;

/**
 * Unwrap Orval's response wrapper type to the raw body type.
 *
 * Orval generates response types like `{ data: T; status: N; headers: Headers }`,
 * but `customInstance` returns the raw JSON body (`T`).  This helper performs
 * the cast so call-sites stay type-safe without scattered `as unknown as`.
 */
export function unwrapResponse<T>(promise: Promise<unknown>): Promise<T> {
	return promise as Promise<T>;
}

/** Error type for generated hooks. */
export type ErrorType<T> = Error & {
	status: number;
	statusText: string;
	data: T | null;
};
