//! Isolated Cargo workspaces for resolver and rule boundary tests.

use crate::workspace::Workspace;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Fixture {
    pub root: PathBuf,
    members: Vec<String>,
}
impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "moat-architecture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create fixture workspace");
        Self {
            root,
            members: Vec::new(),
        }
    }
    pub fn member(&mut self, name: &str, dependencies: &str, source: &str) {
        self.members.push(name.to_string());
        self.write(&format!("{name}/Cargo.toml"), &format!("[package]\nname = {name:?}\nversion = '0.1.0'\nedition = '2021'\n[dependencies]\n{dependencies}\n"));
        self.write(&format!("{name}/src/lib.rs"), source);
    }
    pub fn write(&self, path: &str, source: &str) {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().expect("fixture parent"))
            .expect("create fixture parent");
        std::fs::write(path, source).expect("write fixture file");
    }
    pub fn load(&self) -> Workspace {
        self.write(
            "Cargo.toml",
            &format!(
                "[workspace]\nresolver = '2'\nmembers = {:?}\n",
                self.members
            ),
        );
        Workspace::load(&self.root).expect("load fixture workspace")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove fixture workspace");
    }
}
