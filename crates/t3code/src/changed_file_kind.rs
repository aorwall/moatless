//! `pullRequests.changeBreakdown`'s sort of a changed file by its path alone,
//! ported from T3's `packages/shared/src/changedFileKind.ts`: keep the two in
//! step, or the server's totals disagree with the client's file filters.

/// Which bucket of the breakdown a file's lines count towards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangedFileKind {
    Code,
    Tests,
    Mocks,
    Generated,
    Docs,
}

const GENERATED_FILES: [&str; 20] = [
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "composer.lock",
    "deno.lock",
    "flake.lock",
    "gemfile.lock",
    "go.sum",
    "gradle.lockfile",
    "mix.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "packages.lock.json",
    "pipfile.lock",
    "pnpm-lock.yaml",
    "podfile.lock",
    "poetry.lock",
    "pubspec.lock",
    "uv.lock",
    "yarn.lock",
];
const GENERATED_SUFFIXES: [&str; 10] = [
    ".g.dart",
    ".pb.go",
    "_pb2.py",
    "_pb2_grpc.py",
    ".pb.cc",
    ".pb.h",
    ".min.js",
    ".min.css",
    ".js.map",
    ".css.map",
];
const GENERATED_DIRS: [&str; 3] = ["__generated__", "_generated", "generated"];

const MOCK_WORDS: [&str; 8] = [
    "mock", "mocks", "fixture", "fixtures", "stub", "stubs", "fake", "fakes",
];
const MOCK_DIRS: [&str; 10] = [
    "__fixtures__",
    "__mocks__",
    "__snapshots__",
    "fakes",
    "fixture",
    "fixtures",
    "mock",
    "mocks",
    "stubs",
    "testdata",
];

const TEST_WORDS: [&str; 4] = ["test", "spec", "e2e", "eval"];
const CLASS_TEST_EXTENSIONS: [&str; 9] = [
    "java", "kt", "kts", "scala", "groovy", "cs", "php", "swift", "m",
];
const TEST_DIRS: [&str; 9] = [
    "__tests__",
    "__test__",
    "cypress",
    "e2e",
    "evals",
    "spec",
    "specs",
    "test",
    "tests",
];

const DOCS_EXTENSIONS: [&str; 6] = ["md", "mdx", "markdown", "rst", "adoc", "txt"];

pub(crate) fn classify(path: &str) -> ChangedFileKind {
    let path = path.replace('\\', "/");
    let mut segments: Vec<&str> = path.split('/').collect();
    let name = segments.pop().unwrap_or_default();
    let dirs: Vec<String> = segments.iter().map(|dir| dir.to_lowercase()).collect();
    let in_dir = |set: &[&str]| dirs.iter().any(|dir| set.contains(&dir.as_str()));
    if is_generated_name(name) || in_dir(&GENERATED_DIRS) {
        return ChangedFileKind::Generated;
    }
    if is_mock_name(name) || in_dir(&MOCK_DIRS) {
        return ChangedFileKind::Mocks;
    }
    if is_test_name(name) || is_class_test_name(name) || in_dir(&TEST_DIRS) {
        return ChangedFileKind::Tests;
    }
    if is_docs_name(name) {
        return ChangedFileKind::Docs;
    }
    ChangedFileKind::Code
}

/// `name`'s dot-separated parts, when it has an extension of one or more
/// characters: what `\.x\.[^.]+$` reads as `parts[len - 2] == x`.
fn dotted(name: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = name.split('.').collect();
    (parts.len() >= 2 && !parts[parts.len() - 1].is_empty()).then_some(parts)
}

fn is_generated_name(name: &str) -> bool {
    GENERATED_FILES.contains(&name.to_lowercase().as_str())
        || name.contains(".generated.")
        || name.contains(".gen.")
        || GENERATED_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn is_mock_name(name: &str) -> bool {
    if name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension == "snap")
    {
        return true;
    }
    if dotted(name)
        .is_some_and(|parts| parts.len() >= 3 && MOCK_WORDS.contains(&parts[parts.len() - 2]))
    {
        return true;
    }
    // `^(mocks?|...)([-_.A-Z]|$)`, case-sensitively.
    MOCK_WORDS.iter().any(|word| {
        name.strip_prefix(word).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|next| matches!(next, '-' | '_' | '.') || next.is_ascii_uppercase())
        })
    })
}

fn is_test_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if let Some(parts) = dotted(&lower) {
        let len = parts.len();
        if len >= 3 && TEST_WORDS.contains(&parts[len - 2]) {
            return true;
        }
        let stem = parts[..len - 1].join(".");
        if stem.ends_with("_test") || stem.ends_with("_spec") {
            return true;
        }
        if len == 3 && matches!(parts[0], "vitest" | "jest") && parts[1] == "setup" {
            return true;
        }
    }
    lower == "conftest.py"
        || lower
            .strip_prefix("test_")
            .and_then(|rest| rest.strip_suffix(".py"))
            .is_some_and(|stem| !stem.is_empty())
}

/// `FooTest.java` is a test class, `Latest.java` is not: case-sensitive.
fn is_class_test_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    CLASS_TEST_EXTENSIONS.contains(&extension)
        && ["Test", "Tests", "Spec"]
            .iter()
            .any(|suffix| stem.ends_with(suffix))
}

fn is_docs_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let Some((stem, extension)) = lower.rsplit_once('.') else {
        return false;
    };
    if !DOCS_EXTENSIONS.contains(&extension) {
        return false;
    }
    // Plain-text files that tools read as config or input, not prose.
    let tool_input = extension == "txt"
        && (stem.starts_with("requirements")
            || stem.starts_with("constraints")
            || matches!(stem, "cmakelists" | "robots" | "llms"));
    !tool_input
}

#[cfg(test)]
mod tests {
    use super::ChangedFileKind::{Code, Docs, Generated, Mocks, Tests};
    use super::classify;

    #[test]
    fn files_sort_as_upstream_sorts_them() {
        for (path, kind) in [
            ("src/app.ts", Code),
            ("Cargo.lock", Generated),
            ("api/types.generated.ts", Generated),
            ("web/app.min.js", Generated),
            ("src/__generated__/schema.ts", Generated),
            ("src/user.mock.ts", Mocks),
            ("src/mocks.ts", Mocks),
            ("src/mockData.ts", Mocks),
            ("src/mocking.ts", Code),
            ("tests/fixtures/a.json", Mocks),
            ("src/__snapshots__/a.ts.snap", Mocks),
            ("src/app.test.ts", Tests),
            ("pkg/app_test.go", Tests),
            ("test_app.py", Tests),
            ("conftest.py", Tests),
            ("vitest.setup.ts", Tests),
            ("src/FooTest.java", Tests),
            ("src/Latest.java", Code),
            ("tests/integration.rs", Tests),
            ("README.md", Docs),
            ("notes.txt", Docs),
            ("requirements-dev.txt", Code),
            ("CMakeLists.txt", Code),
        ] {
            assert_eq!(classify(path), kind, "{path}");
        }
    }
}
