//! Atomic JSON writer for the sketchybar state file.

use std::path::Path;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

pub trait StateWriter: Send {
    fn write(&mut self, value: &serde_json::Value) -> Result<()>;
}

pub struct FileStateWriter {
    path: std::path::PathBuf,
}

impl FileStateWriter {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl StateWriter for FileStateWriter {
    fn write(&mut self, value: &serde_json::Value) -> Result<()> {
        write_state_json(&self.path, value)
    }
}

pub fn write_state_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating tempfile in {}", dir.display()))?;
    serde_json::to_writer(&mut tmp, value)?;
    tmp.persist(path)
        .with_context(|| format!("persisting {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn writes_and_replaces_atomically() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        write_state_json(&path, &json!({"a": 1})).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert_eq!(s, r#"{"a":1}"#);

        write_state_json(&path, &json!({"b": 2})).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        assert_eq!(s, r#"{"b":2}"#);
    }

    #[test]
    fn creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested/deep/state.json");
        write_state_json(&path, &json!({"ok": true})).unwrap();
        assert!(path.exists());
    }
}
