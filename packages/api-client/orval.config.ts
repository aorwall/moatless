import { defineConfig } from "orval";

export default defineConfig({
	sandbox: {
		input: {
			target: "../../openapi-specs.json",
		},
		output: {
			target: "./src/sandbox/index.ts",
			schemas: "./src/sandbox/model",
			client: "react-query",
			mode: "tags-split",
			override: {
				mutator: {
					path: "./custom-instance.ts",
					name: "customInstance",
				},
				query: {
					useQuery: true,
					useMutation: true,
					usePrefetch: true,
					signal: true,
				},
			},
		},
	},
});
