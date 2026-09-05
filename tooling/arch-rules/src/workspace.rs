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
    pub targets: Vec<(PathBuf, bool)>,
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
                    if kinds.iter().any(|k| k == "custom-build") {
                        return None;
                    }
                    let test = kinds
                        .iter()
                        .any(|k| k == "test" || k == "bench" || k == "example");
                    Some((path, test))
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

        Ok(Self {
            root,
            members,
            index: OnceCell::new(),
        })
    }

    /// The parsed, resolved source inventory shared by the rules in this run.
    pub fn index(&self) -> Result<&crate::source_index::Index> {
        self.index
            .get_or_init(|| crate::source_index::Index::read(self).map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow!("{e}"))
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
        let src = member.dir.join("src");
        if !src.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        for entry in WalkDir::new(&src).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") {
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
