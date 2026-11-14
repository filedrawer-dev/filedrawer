//! The repo's `skills/` folder, for the tests of the skills, their scripts and their
//! git tags.

use std::path::{Path, PathBuf};

pub const NAMES: [&str; 2] = ["filedrawer-search", "filedrawer-submit"];

pub fn skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills")
}

/// A skill's file as the repo holds it, naming production.
pub fn raw(name: &str, file: &str) -> String {
    let path = skills_dir().join(name).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `metadata.version` of a skill, from its SKILL.md in the repo.
pub fn version(name: &str) -> String {
    metadata(name, "version")
}

/// The files a skill's `metadata.files` lists.
pub fn files(name: &str) -> Vec<String> {
    metadata(name, "files")
        .split(' ')
        .map(str::to_string)
        .collect()
}

fn metadata(name: &str, key: &str) -> String {
    raw(name, "SKILL.md")
        .lines()
        .find_map(|line| line.strip_prefix(&format!("  {key}: \"")))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{name} has no metadata.{key}"))
        .to_string()
}

/// A folder under `root` laid out as raw.githubusercontent.com serves the repo's tags,
/// holding each skill's tag at its version, with `edit` applied to each file's text,
/// given its skill and path.
pub fn tags_mirror(root: &Path, edit: impl Fn(&str, &str, String) -> String) -> PathBuf {
    for name in NAMES {
        let tag = root.join(format!(
            "refs/tags/skill-{name}-v{}/skills/{name}",
            version(name)
        ));
        for file in files(name) {
            let target = tag.join(&file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, edit(name, &file, raw(name, &file))).unwrap();
        }
    }
    root.to_path_buf()
}
