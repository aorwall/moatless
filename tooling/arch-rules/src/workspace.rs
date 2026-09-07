//! The workspace a rule reads: its member crates, their normal dependencies,
//! and the source files under each one's `src/`.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use walkdir::WalkDir;

/// One workspace member.
pub struct Member {
    /// The package name, e.g. `moatless-types`.
    pub name: String,
    /// The directory holding its `Cargo.toml`.
    pub dir: PathBuf,
    /// Normal (non-dev, non-build) dependency names, in-workspace and external.
    pub normal_deps: BTreeSet<String>,
    pub dependencies: Vec<Dependency>,
    pub manifest: toml::Table,
    pub metadata: serde_json::Value,
    pub targets: Vec<Target>,
}

/// A Cargo target and its source role.
pub struct Target {
    pub path: PathBuf,
    pub kinds: Vec<String>,
}

impl Target {
    pub(crate) fn test_only(&self) -> bool {
        self.kinds
            .iter()
            .any(|k| matches!(k.as_str(), "test" | "bench" | "example" | "custom-build"))
    }

    pub(crate) fn library(&self) -> bool {
        self.kinds.iter().any(|k| {
            matches!(
                k.as_str(),
                "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
            )
        })
    }

    pub(crate) fn executable(&self) -> bool {
        self.kinds.iter().any(|k| k == "bin")
    }
}

/// One declared dependency, before Cargo unifies features.
pub struct Dependency {
    pub name: String,
    pub alias: String,
    pub kind: Option<String>,
    pub target: Option<String>,
    pub optional: bool,
    pub features: Vec<String>,
}

pub struct Workspace {
    pub root: PathBuf,
    members: BTreeMap<String, Member>,
    index: OnceCell<Result<crate::source_index::Index, String>>,
    source_only: Vec<Member>,
    role_index: OnceCell<Result<crate::source_index::Index, String>>,
}

impl Workspace {
    /// Read the workspace by shelling out to `cargo metadata`.
    ///
    /// `--no-deps` keeps this to the members and their declared dependency
    /// names, which is all any rule here needs, and keeps the call fast enough
    /// to run on every test invocation.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .current_dir(&root)
            .output()
            .context("running `cargo metadata`")?;
        if !out.status.success() {
            return Err(anyhow!(
                "`cargo metadata` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        let json: serde_json::Value =
            serde_json::from_slice(&out.stdout).context("parsing `cargo metadata` output")?;
        let packages = json["packages"]
            .as_array()
            .ok_or_else(|| anyhow!("`cargo metadata` returned no packages array"))?;

        let mut members = BTreeMap::new();
        for pkg in packages {
            let name = pkg["name"]
                .as_str()
                .ok_or_else(|| anyhow!("a package has no name"))?
                .to_string();
            let manifest = pkg["manifest_path"]
                .as_str()
                .ok_or_else(|| anyhow!("package {name} has no manifest_path"))?;
            let dir = Path::new(manifest)
                .parent()
                .ok_or_else(|| anyhow!("manifest {manifest} has no parent"))?
                .to_path_buf();

            let normal_deps = pkg["dependencies"]
                .as_array()
                .map(|deps| {
                    deps.iter()
                        .filter(|d| d["kind"].is_null())
                        .filter_map(|d| d["name"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            let dependencies = pkg["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    let name = d["name"].as_str().unwrap_or_default().to_string();
                    Dependency {
                        alias: d["rename"].as_str().unwrap_or(&name).to_string(),
                        name,
                        kind: d["kind"].as_str().map(str::to_string),
                        target: d["target"].as_str().map(str::to_string),
                        optional: d["optional"].as_bool().unwrap_or(false),
                        features: d["features"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|f| f.as_str().map(str::to_string))
                            .collect(),
                    }
                })
                .collect();
            let raw = std::fs::read_to_string(manifest)?;
            let manifest = raw
                .parse::<toml::Table>()
                .context("parsing member manifest")?;
            let targets = pkg["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    let path = PathBuf::from(t["src_path"].as_str()?);
                    let kinds = t["kind"].as_array()?;
                    Some(Target {
                        path,
                        kinds: kinds
                            .iter()
                            .filter_map(|k| k.as_str().map(str::to_owned))
                            .collect(),
                    })
                })
                .collect();

            members.insert(
                name.clone(),
                Member {
                    name,
                    dir,
                    normal_deps,
                    dependencies,
                    manifest,
                    metadata: pkg["metadata"].clone(),
                    targets,
                },
            );
        }

        let source_only = source_only_members(&root)?;
        Ok(Self {
            root,
            members,
            index: OnceCell::new(),
            source_only,
            role_index: OnceCell::new(),
        })
    }

    /// The parsed, resolved source inventory shared by the rules in this run.
    pub(crate) fn index(&self) -> Result<&crate::source_index::Index> {
        self.index
            .get_or_init(|| crate::source_index::Index::read(self).map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!("{e}"))
    }

    pub(crate) fn role_index(&self) -> Result<&crate::source_index::Index> {
        self.role_index
            .get_or_init(|| {
                crate::source_index::Index::read_members(self, self.source_members(), false)
                    .map_err(|e| format!("{e:#}"))
            })
            .as_ref()
            .map_err(|e| anyhow!("{e}"))
    }

    pub(crate) fn source_members(&self) -> impl Iterator<Item = &Member> {
        self.members().chain(self.source_only.iter())
    }

    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.get(name)
    }

    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// Whether `from` reaches `to` through normal in-workspace dependencies.
    ///
    /// Only in-workspace edges are walked, so an external crate is answered by
    /// direct declaration alone — which is the question the sqlx rule asks.
    pub fn reaches(&self, from: &str, to: &str) -> bool {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(from.to_string());
        while let Some(current) = queue.pop_front() {
            let Some(member) = self.members.get(&current) else {
                continue;
            };
            for dep in &member.normal_deps {
                if dep == to {
                    return true;
                }
                if self.members.contains_key(dep) && seen.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }
        false
    }

    /// Every `.rs` file under a member's `src/`, as (relative path, contents).
    ///
    /// Paths are relative to the workspace root so a violation reads the same
    /// wherever the checkout lives.
    pub fn sources(&self, member: &Member) -> Result<Vec<(String, String)>> {
        let mut roots = BTreeSet::from([member.dir.join("src")]);
        roots.extend(
            member
                .targets
                .iter()
                .filter_map(|t| t.path.parent().map(Path::to_path_buf)),
        );
        let mut files = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in roots
            .iter()
            .filter(|p| p.is_dir())
            .flat_map(|root| WalkDir::new(root).into_iter())
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") && seen.insert(path.to_path_buf()) {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                files.push((self.relative(path), text));
            }
        }
        files.sort();
        Ok(files)
    }

    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// Find the workspace root from the crate this code was compiled in.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn source_only_members(root: &Path) -> Result<Vec<Member>> {
    // The excluded SDK depends on an external Codex checkout. Read its source
    // without resolving that checkout so CI still enforces file responsibilities.
    let dir = root.join("crates/codex-sdk");
    let path = dir.join("Cargo.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let manifest: toml::Table = std::fs::read_to_string(path)?.parse()?;
    let name = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .context("source-only SDK package name")?
        .to_owned();
    let mut targets = Vec::new();
    let library = manifest
        .get("lib")
        .and_then(|v| v.get("path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("src/lib.rs");
    if dir.join(library).exists() {
        targets.push(Target {
            path: dir.join(library),
            kinds: vec!["lib".into()],
        });
    }
    if dir.join("src/main.rs").exists() {
        targets.push(Target {
            path: dir.join("src/main.rs"),
            kinds: vec!["bin".into()],
        });
    }
    for bin in manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = bin.get("path").and_then(toml::Value::as_str) {
            targets.push(Target {
                path: dir.join(path),
                kinds: vec!["bin".into()],
            });
        }
    }
    Ok(vec![Member {
        name,
        dir,
        normal_deps: BTreeSet::new(),
        dependencies: Vec::new(),
        metadata: serde_json::Value::Null,
        manifest,
        targets,
    }])
}
