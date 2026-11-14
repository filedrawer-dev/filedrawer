//! The two skills `/skill.md` installs (design.md, Skills), compiled in from the
//! repo's `skills/` folder, and the versions `/skill.md` names in its
//! `X-Filedrawer-Skills` header.

use std::sync::LazyLock;

/// One skill's folder, as this build has it.
pub struct Skill {
    pub name: &'static str,
    /// Each file in the folder, by its path there.
    files: &'static [(&'static str, &'static str)],
}

macro_rules! skill {
    ($name:literal) => {
        Skill {
            name: $name,
            files: &[
                (
                    "SKILL.md",
                    include_str!(concat!("../../../skills/", $name, "/SKILL.md")),
                ),
                (
                    "references/entry-format.md",
                    include_str!(concat!(
                        "../../../skills/",
                        $name,
                        "/references/entry-format.md"
                    )),
                ),
                (
                    "references/upgrade.md",
                    include_str!(concat!("../../../skills/", $name, "/references/upgrade.md")),
                ),
            ],
        }
    };
}

pub const SKILLS: [Skill; 2] = [skill!("filedrawer-search"), skill!("filedrawer-submit")];

/// The response header that names each skill's version.
pub const VERSIONS_HEADER: &str = "x-filedrawer-skills";

/// Each skill's `metadata.version`, read once. A build whose SKILL.md lacks one
/// panics on first use, which `build` makes the server's start.
static VERSIONS: LazyLock<Vec<(&'static str, u32)>> = LazyLock::new(|| {
    SKILLS
        .iter()
        .map(|skill| (skill.name, skill.read_version()))
        .collect()
});

impl Skill {
    pub fn named(name: &str) -> Option<&'static Skill> {
        SKILLS.iter().find(|skill| skill.name == name)
    }

    /// The file at `path` in the folder, as stored, naming production.
    pub fn file(&self, path: &str) -> Option<&'static str> {
        self.files
            .iter()
            .find(|(file, _)| *file == path)
            .map(|(_, text)| *text)
    }

    fn read_version(&self) -> u32 {
        let text = self.file("SKILL.md").unwrap_or_default();
        text.lines()
            .find_map(|line| line.strip_prefix("  version: \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .and_then(|digits| digits.parse().ok())
            .unwrap_or_else(|| panic!("{}'s SKILL.md has no metadata.version", self.name))
    }
}

/// Each skill as `name/version`, joined by `separator`.
pub fn versions(separator: &str) -> String {
    VERSIONS
        .iter()
        .map(|(name, version)| format!("{name}/{version}"))
        .collect::<Vec<_>>()
        .join(separator)
}

/// Each skill's folder at its git tag on GitHub, for the installer to show.
pub fn tag_urls() -> String {
    VERSIONS
        .iter()
        .map(|(name, version)| {
            format!(
                "`https://github.com/filedrawer-dev/filedrawer/tree/skill-{name}-v{version}/skills/{name}`"
            )
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The site the skills' files name.
const PRODUCTION: &str = "https://filedrawer.dev";

/// Whether the site at `public_url` is the one the skills' files name.
pub fn is_production(public_url: &str) -> bool {
    public_url == PRODUCTION
}

/// A skill's file, or the installer, pointed at the site at `public_url`, as the
/// installer and the upgrade steps point a skill's files: `https://filedrawer.dev`
/// becomes `public_url`, and the header file `~/.config/filedrawer/filedrawer.dev.header`
/// becomes that site's. On production it changes nothing.
pub fn for_site(text: &str, public_url: &str) -> String {
    let host = crate::routes::files::host_of(public_url);
    text.replace(PRODUCTION, public_url)
        .replace("/filedrawer.dev.header", &format!("/{host}.header"))
}
