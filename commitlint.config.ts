import { readdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Directory names under a repo-relative path, kebab-case by convention here.
 *
 * Deriving the code scopes rather than listing them keeps a new crate or
 * package valid the day it lands: a hardcoded list went stale once already and
 * left every `crates/` change unable to carry any scope at all.
 */
const directoryScopes = (relativePath: string): string[] =>
	readdirSync(join(import.meta.dirname, relativePath), { withFileTypes: true })
		.filter((entry) => entry.isDirectory() && !entry.name.startsWith("."))
		.map((entry) => entry.name);

// Scopes that name no single directory, taken from the scopes `main`'s history
// actually uses. `features`/`layout`/`routes`/`styles`/`utils`/`hooks` are the
// T3 UI's own vocabulary and predate the rest.
const NON_DIRECTORY_SCOPES = [
	"adapters",
	"backend",
	"biome",
	"ci",
	"deps",
	"docker",
	"docs",
	"e2e",
	"features",
	"frontend",
	"helm",
	"hooks",
	"k8s",
	"layout",
	"migration",
	"routes",
	"scripts",
	"skills",
	"styles",
	"tests",
	"tooling",
	"ui-rpc",
	"utils",
];

const SCOPES = [
	...new Set([
		...directoryScopes("crates"),
		...directoryScopes("packages"),
		...NON_DIRECTORY_SCOPES,
	]),
].sort();

/** @type {import('@commitlint/types').UserConfig} */
const CommitLintConfiguration = {
	extends: ["@commitlint/config-conventional"],
	rules: {
		// Not `kebab-case`: that check compares against lodash's kebabCase, which
		// splits letter/digit boundaries, so it rejects the crate names `t3code`
		// and `s3` outright. The enum below already pins each scope's spelling.
		"scope-case": [2, "always", "lower-case"],
		// A scope stays optional: an empty scope is not checked against the enum.
		"scope-enum": [2, "always", SCOPES],
	},
};

export default CommitLintConfiguration;
