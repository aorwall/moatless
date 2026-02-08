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
		throw Object.assign(new Error(response.statusText), {
			status: response.status,
			statusText: response.statusText,
			data: errorData,
		});
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
