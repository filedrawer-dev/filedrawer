//! Seam: the repo's `skills/` folder as files (design.md, Skills). Each skill's
//! version and folder hash are on record in `skills/versions.txt`, its front matter
//! keeps to the Agent Skills standard, and every command it gives the agent sends the
//! header file and names the skill.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::skills::{NAMES, raw, skills_dir, version};
use sha2::{Digest, Sha256};

const FIX: &str = "run bin/skills sync";

/// The files of a skill an agent follows: SKILL.md and its upgrade steps.
const FOLLOWED: [&str; 2] = ["SKILL.md", "references/upgrade.md"];

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn skill_md(name: &str) -> String {
    raw(name, "SKILL.md")
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Each file under `dir`, as `./path`, in byte order.
fn files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &name, out);
            } else {
                out.push(name);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, ".", &mut out);
    out.sort();
    out
}

/// The folder's hash, as `find . -type f | LC_ALL=C sort | xargs sha256sum | sha256sum`
/// prints it from inside the folder.
fn folder_hash(dir: &Path) -> String {
    let listing: String = files(dir)
        .iter()
        .map(|file| {
            let bytes = std::fs::read(dir.join(file)).unwrap();
            format!("{}  {file}\n", hex(&bytes))
        })
        .collect();
    hex(listing.as_bytes())
}

/// The value of `key:` in the front matter, a line of its own.
fn front(text: &str, key: &str) -> Option<String> {
    let body = text.strip_prefix("---\n")?;
    let matter = &body[..body.find("\n---\n")?];
    matter
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .map(|value| value.trim_matches('"').to_string())
}

/// Each `curl` command in `text`, its continued lines joined.
fn curls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if !line.contains("curl ") {
            continue;
        }
        let mut command = line.to_string();
        while command.ends_with('\\') {
            command.pop();
            command += lines.next().unwrap_or_default();
        }
        out.push(command);
    }
    out
}

#[test]
fn skills_holds_these_two_skills_and_the_record() {
    let mut found: Vec<String> = std::fs::read_dir(skills_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(
        found,
        ["filedrawer-search", "filedrawer-submit", "versions.txt"]
    );
}

#[test]
fn each_folder_is_on_record_at_its_version() {
    let record = read(&skills_dir().join("versions.txt"));
    for name in NAMES {
        let line = record
            .lines()
            .find(|line| line.split(' ').next() == Some(name))
            .unwrap_or_else(|| panic!("versions.txt has no line for {name}: {FIX}"));
        let fields: Vec<&str> = line.split(' ').collect();
        assert_eq!(fields.len(), 3, "{line}");
        assert_eq!(
            fields[1],
            version(name),
            "{name}'s version isn't the one versions.txt records: {FIX}"
        );
        assert_eq!(
            fields[2],
            folder_hash(&skills_dir().join(name)),
            "skills/{name} changed since versions.txt recorded it: {FIX}"
        );
    }
}

#[test]
fn metadata_files_lists_the_folders_files() {
    for name in NAMES {
        let listed: BTreeSet<String> = front(&skill_md(name), "  files")
            .expect("metadata.files")
            .split(' ')
            .map(|file| format!("./{file}"))
            .collect();
        let found: BTreeSet<String> = files(&skills_dir().join(name)).into_iter().collect();
        assert_eq!(listed, found, "{name}");
    }
}

#[test]
fn each_skill_bundles_the_entry_format_as_it_stands() {
    let format = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/entry-format.md"));
    for name in NAMES {
        let bundled = read(&skills_dir().join(name).join("references/entry-format.md"));
        assert!(
            bundled == format,
            "{name}'s references/entry-format.md is out of date: {FIX}"
        );
    }
}

#[test]
fn front_matter_keeps_to_the_agent_skills_standard() {
    for name in NAMES {
        let text = skill_md(name);
        assert_eq!(front(&text, "name").as_deref(), Some(name));
        assert!(name.len() <= 64);
        assert!(
            name.split('-').all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
            "{name}"
        );
        // Rendered for the longest site address the server runs at.
        let description = front(&text, "description")
            .expect("description")
            .replace("{{BASE}}", "https://staging.filedrawer.dev");
        assert!(
            description.chars().count() <= 1024,
            "{name}: {} characters",
            description.len()
        );
        assert!(version(name).parse::<u32>().is_ok_and(|v| v >= 1), "{name}");
    }
}

#[test]
fn every_request_to_the_site_sends_the_header_file_and_names_the_skill() {
    for name in NAMES {
        let text = FOLLOWED.map(|file| raw(name, file)).join("\n");
        let skill_header = format!("-H \"X-Filedrawer-Skill: {name}/{}\"", version(name));
        let commands: Vec<String> = curls(&text)
            .into_iter()
            .filter(|command| command.contains("{{BASE}}"))
            .collect();
        assert!(!commands.is_empty(), "{name}");
        for command in commands {
            assert!(
                command.contains("-H @\"$HOME/.config/filedrawer/{{HOST}}.header\""),
                "{name}: {command}"
            );
            assert!(command.contains(&skill_header), "{name}: {command}");
        }
        for (at, _) in text.match_indices("X-Filedrawer-Skill: ") {
            let named = &text[at..];
            let expected = format!("X-Filedrawer-Skill: {name}/{}\"", version(name));
            assert!(
                named.starts_with(&expected),
                "{name}: {}: {FIX}",
                &named[..40]
            );
        }
    }
}

#[test]
fn only_the_token_setup_reads_filedrawer_token() {
    for name in NAMES {
        for file in FOLLOWED {
            let text = raw(name, file);
            let setup = text
                .find("### The token\n")
                .map(|start| start..start + text[start..].find("\n### The format\n").unwrap());
            for (at, _) in text.match_indices("FILEDRAWER_TOKEN") {
                assert!(
                    setup.as_ref().is_some_and(|setup| setup.contains(&at)),
                    "{name}/{file} reads $FILEDRAWER_TOKEN outside The token"
                );
            }
        }
    }
}

#[test]
fn the_skills_send_the_agent_to_no_instructions_on_the_site() {
    for name in NAMES {
        let text = FOLLOWED.map(|file| raw(name, file)).join("\n");
        assert!(!text.contains("{{BASE}}/entry-format.md"), "{name}");
        for (at, _) in text.match_indices("{{BASE}}/skill.md") {
            let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            let line = &text[line_start..at];
            assert!(
                line.contains("curl -sSI "),
                "{name} names /skill.md outside the upgrade check"
            );
        }
    }
}

#[test]
fn both_skills_upgrade_the_same_way() {
    let upgrades = |name: &str| {
        let text = skill_md(name);
        let start = text.find("## Upgrades\n").expect("an Upgrades section");
        let end = text[start + 1..]
            .find("\n## ")
            .map_or(text.len(), |i| start + 1 + i);
        let version_named = format!("/{}\"", version(name));
        let section = text[start..end].replace(&version_named, "/N\"");
        NAMES
            .iter()
            .fold(section, |section, name| section.replace(name, "NAME"))
    };
    assert_eq!(upgrades("filedrawer-search"), upgrades("filedrawer-submit"));
    let steps = |name: &str| raw(name, "references/upgrade.md").replace(name, "NAME");
    assert_eq!(steps("filedrawer-search"), steps("filedrawer-submit"));
}
