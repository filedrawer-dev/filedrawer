//! Splits an entry into front matter and body, and reads the front matter's fields
//! (docs/entry-format.md, Front matter).

use yaml_rust2::scanner::{Scanner, TokenType};
use yaml_rust2::{Yaml, YamlLoader};

use crate::choice::{Choice, Vocabulary};
use crate::fields::{DataItem, Fields, Model, PositiveControl, SmallestEffect, Writer};
use crate::report::{Refusal, Warning, WarningKind};

/// The front matter's fields, the names it didn't recognise, and what it found wrong.
pub(crate) struct FrontMatter {
    pub fields: Fields,
    pub unrecognised: Vec<String>,
    pub refusals: Vec<Refusal>,
    pub warnings: Vec<Warning>,
}

/// Splits `text` (newlines already normalised) at the closing `---` line. Returns the
/// YAML and the body, or `None` when the entry doesn't start with front matter.
pub(crate) fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    if let Some(body) = rest.strip_prefix("---\n") {
        return Some(("", body));
    }
    let end = rest.find("\n---\n").map(|at| (at, at + 5)).or_else(|| {
        rest.strip_suffix("\n---")
            .map(|yaml| (yaml.len(), rest.len()))
    })?;
    Some((&rest[..end.0], &rest[end.1..]))
}

/// Reads the front matter's YAML.
pub(crate) fn read(yaml: &str) -> FrontMatter {
    let mut reader = Reader::default();
    if uses_aliases(yaml) {
        reader.refusals.push(Refusal::FrontMatterUnreadable(
            "it uses a YAML alias (`*name`); write each value out in full".into(),
        ));
        return reader.finish();
    }
    match load(yaml) {
        Ok(map) => {
            reader.flow_mappings(yaml);
            reader.fields(&map);
        }
        Err(why) => reader.refusals.push(Refusal::FrontMatterUnreadable(why)),
    }
    reader.finish()
}

/// Whether the YAML refers to an anchor. The loader expands aliases, so a few hundred
/// bytes of nested aliases would expand to gigabytes; the scanner alone doesn't.
fn uses_aliases(yaml: &str) -> bool {
    Scanner::new(yaml.chars()).any(|token| matches!(token.1, TokenType::Alias(_)))
}

fn load(yaml: &str) -> Result<yaml_rust2::yaml::Hash, String> {
    let documents = YamlLoader::load_from_str(yaml).map_err(|error| error.to_string())?;
    match documents.into_iter().next() {
        None | Some(Yaml::Null) => Ok(Default::default()),
        Some(Yaml::Hash(map)) => Ok(map),
        Some(_) => Err("it is not a mapping of field names to values".into()),
    }
}

#[derive(Default)]
struct Reader {
    fields: Fields,
    unrecognised: Vec<String>,
    refusals: Vec<Refusal>,
    warnings: Vec<Warning>,
}

impl Reader {
    fn finish(self) -> FrontMatter {
        FrontMatter {
            fields: self.fields,
            unrecognised: self.unrecognised,
            refusals: self.refusals,
            warnings: self.warnings,
        }
    }

    fn warn(&mut self, kind: WarningKind, message: String) {
        self.warnings.push(Warning::new(kind, message));
    }

    /// Warns once for each `{a, b}` mapping, which splits a value with a comma into
    /// extra keys without an error.
    fn flow_mappings(&mut self, yaml: &str) {
        for token in Scanner::new(yaml.chars()) {
            if token.1 == TokenType::FlowMappingStart {
                self.warn(
                    WarningKind::FlowMapping,
                    format!(
                        "line {} of the front matter uses `{{a, b}}` style; a comma inside it \
                         splits a value into extra keys, so write nested values in block style",
                        token.0.line()
                    ),
                );
            }
        }
    }

    fn fields(&mut self, map: &yaml_rust2::yaml::Hash) {
        for (key, value) in map {
            let name = match key {
                Yaml::String(name) => name.as_str(),
                other => {
                    let name = scalar_text(other).unwrap_or_default();
                    self.warn(
                        WarningKind::MalformedField,
                        format!("the front-matter key `{name}` is not text"),
                    );
                    continue;
                }
            };
            self.field(name, value);
        }
        let fields = &self.fields;
        let required = [
            ("title", fields.title.is_some()),
            ("safety_motivation", fields.safety_motivation.is_some()),
            ("hypothesis", fields.hypothesis.is_some()),
            ("outcome", fields.outcome != Choice::Absent),
        ];
        for (name, present) in required {
            if !present {
                self.refusals.push(Refusal::MissingField(name));
            }
        }
    }

    fn field(&mut self, name: &str, value: &Yaml) {
        match name {
            "schema" => self.fields.schema = self.text(name, value),
            "title" => self.fields.title = self.text(name, value),
            "safety_motivation" => self.fields.safety_motivation = self.text(name, value),
            "hypothesis" => self.fields.hypothesis = self.text(name, value),
            "outcome" => self.fields.outcome = self.choice(value),
            "kind" => self.fields.kind = self.choice(value),
            "predicted_direction" => self.fields.predicted_direction = self.choice(value),
            "hypothesis_set" => self.fields.hypothesis_set = self.choice(value),
            "hypothesis_source" => self.fields.hypothesis_source = self.text(name, value),
            "stage" => self.fields.stage = self.choice(value),
            "stopped_because" => self.fields.stopped_because = self.text(name, value),
            "superseded_by" => self.fields.superseded_by = self.text(name, value),
            "run_dates" => self.fields.run_dates = self.list(name, value),
            "models" => self.fields.models = self.mapping_list(name, value, Self::model),
            "tags" => self.fields.tags = self.list(name, value),
            "positive_control" => {
                self.fields.positive_control = self.mapping(name, value, Self::positive_control)
            }
            "smallest_effect_of_interest" => {
                self.fields.smallest_effect_of_interest =
                    self.mapping(name, value, Self::smallest_effect)
            }
            "code" => self.fields.code = self.text(name, value),
            "code_state" => self.fields.code_state = self.choice(value),
            "code_license" => self.fields.code_license = self.text(name, value),
            "data" => self.fields.data = self.mapping_list(name, value, Self::data_item),
            "logs" => self.fields.logs = self.list(name, value),
            "replicates" => self.fields.replicates = self.paper(name, value),
            "reanalyzes" => self.fields.reanalyzes = self.paper(name, value),
            "uses_runs" => self.fields.uses_runs = self.list(name, value),
            "published_in" => self.fields.published_in = self.published_in(value),
            "slug" => self.fields.slug = self.text(name, value),
            "series" => self.fields.series = self.text(name, value),
            "written_by" => self.fields.written_by = self.mapping_list(name, value, Self::writer),
            "embargo_until" => self.fields.embargo_until = self.text(name, value),
            "label" => self.warn(
                WarningKind::StrippedField,
                "the server removed `label`; cards come from the analyzer, not the entry".into(),
            ),
            reserved if reserved.starts_with("fd_") => self.warn(
                WarningKind::StrippedField,
                format!("the server removed `{reserved}`; the `fd_` prefix is reserved for its own fields"),
            ),
            other => {
                self.unrecognised.push(other.to_string());
                self.warn(
                    WarningKind::UnrecognisedField,
                    format!("`{other}` is not a field the server knows; it is stored as written"),
                );
            }
        }
    }

    /// A scalar as text. Empty text, `null` and collections read as absent; a
    /// collection also warns.
    fn text(&mut self, name: &str, value: &Yaml) -> Option<String> {
        match value {
            Yaml::Array(_) | Yaml::Hash(_) => {
                self.warn(
                    WarningKind::MalformedField,
                    format!("`{name}` should be a single value, not a list or mapping"),
                );
                None
            }
            scalar => scalar_text(scalar),
        }
    }

    /// A field that names a paper or entry, where YAML reads a bare arXiv ID as a number.
    fn paper(&mut self, name: &str, value: &Yaml) -> Option<String> {
        if matches!(value, Yaml::Real(_) | Yaml::Integer(_)) {
            let text = scalar_text(value).unwrap_or_default();
            self.warn(
                WarningKind::NumberForPaper,
                format!(
                    "`{name}: {text}` reads as a number in YAML; write an arXiv paper as \
                     `arXiv:<id>`, or give a DOI"
                ),
            );
        }
        self.text(name, value)
    }

    fn published_in(&mut self, value: &Yaml) -> Option<String> {
        let text = self.paper("published_in", value)?;
        if !is_paper_id(&text) && !matches!(value, Yaml::Real(_) | Yaml::Integer(_)) {
            self.warn(
                WarningKind::NotAPaper,
                format!("`published_in: {text}` is not a DOI or `arXiv:<id>`; other links go in the body"),
            );
        }
        Some(text)
    }

    fn choice<T: Vocabulary>(&mut self, value: &Yaml) -> Choice<T> {
        let Some(text) = scalar_text(value) else {
            if matches!(value, Yaml::Array(_) | Yaml::Hash(_)) {
                self.warn(
                    WarningKind::MalformedField,
                    format!("`{}` should be a single value", T::FIELD),
                );
            }
            return Choice::Absent;
        };
        match T::parse(text.trim()) {
            Some(known) => Choice::Known(known),
            None => {
                self.warn(
                    WarningKind::UnrecognisedValue,
                    format!("`{}: {text}` is not a value the server knows", T::FIELD),
                );
                Choice::Unrecognised(text)
            }
        }
    }

    /// A list of scalars. A single scalar reads as a list of one.
    fn list(&mut self, name: &str, value: &Yaml) -> Vec<String> {
        match value {
            Yaml::Array(items) => items
                .iter()
                .filter_map(|item| self.text(name, item))
                .collect(),
            Yaml::Hash(_) => {
                self.warn(
                    WarningKind::MalformedField,
                    format!("`{name}` should be a list"),
                );
                Vec::new()
            }
            scalar => scalar_text(scalar).into_iter().collect(),
        }
    }

    /// A list of mappings, each read by `item`.
    fn mapping_list<T>(
        &mut self,
        name: &str,
        value: &Yaml,
        item: fn(&mut Self, &yaml_rust2::yaml::Hash) -> T,
    ) -> Vec<T> {
        let Yaml::Array(entries) = value else {
            if *value != Yaml::Null {
                self.warn(
                    WarningKind::MalformedField,
                    format!("`{name}` should be a list with one item per line, each a mapping"),
                );
            }
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| match entry {
                Yaml::Hash(map) => Some(item(self, map)),
                _ => {
                    self.warn(
                        WarningKind::MalformedField,
                        format!("each item of `{name}` should be a mapping, such as `- name: ...`"),
                    );
                    None
                }
            })
            .collect()
    }

    fn mapping<T>(
        &mut self,
        name: &str,
        value: &Yaml,
        read: fn(&mut Self, &yaml_rust2::yaml::Hash) -> T,
    ) -> Option<T> {
        match value {
            Yaml::Hash(map) => Some(read(self, map)),
            Yaml::Null => None,
            _ => {
                self.warn(
                    WarningKind::MalformedField,
                    format!("`{name}` should be a mapping"),
                );
                None
            }
        }
    }

    fn model(&mut self, map: &yaml_rust2::yaml::Hash) -> Model {
        Model {
            name: self.text("models.name", get(map, "name")),
            role: self.choice(get(map, "role")),
            route: self.text("models.route", get(map, "route")),
        }
    }

    fn positive_control(&mut self, map: &yaml_rust2::yaml::Hash) -> PositiveControl {
        PositiveControl {
            outcome: self.choice(get(map, "outcome")),
            checks: self.choice(get(map, "checks")),
            source: self.choice(get(map, "source")),
        }
    }

    fn smallest_effect(&mut self, map: &yaml_rust2::yaml::Hash) -> SmallestEffect {
        SmallestEffect {
            value: self.text("smallest_effect_of_interest.value", get(map, "value")),
            set: self.choice(get(map, "set")),
        }
    }

    fn data_item(&mut self, map: &yaml_rust2::yaml::Hash) -> DataItem {
        DataItem {
            url: self.text("data.url", get(map, "url")),
            sha256: self.text("data.sha256", get(map, "sha256")),
        }
    }

    fn writer(&mut self, map: &yaml_rust2::yaml::Hash) -> Writer {
        let writer = Writer {
            model: self.text("written_by.model", get(map, "model")),
            effort: self.text("written_by.effort", get(map, "effort")),
            temperature: self.text("written_by.temperature", get(map, "temperature")),
            harness: self.text("written_by.harness", get(map, "harness")),
        };
        if writer.model.is_none() {
            self.warn(
                WarningKind::MalformedField,
                "an item of `written_by` has no `model`".into(),
            );
        }
        writer
    }
}

static NULL: Yaml = Yaml::Null;

fn get<'a>(map: &'a yaml_rust2::yaml::Hash, key: &str) -> &'a Yaml {
    map.get(&Yaml::String(key.into())).unwrap_or(&NULL)
}

/// A scalar's text as written; `None` for null, empty text and collections.
fn scalar_text(value: &Yaml) -> Option<String> {
    let text = match value {
        Yaml::String(text) | Yaml::Real(text) => text.clone(),
        Yaml::Integer(number) => number.to_string(),
        Yaml::Boolean(flag) => flag.to_string(),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Whether `text` is `arXiv:<id>` or a DOI (`10.<registrant>/<suffix>`, bare or as a
/// doi.org URL).
pub(crate) fn is_paper_id(text: &str) -> bool {
    let text = text.trim();
    if let Some(id) = text.strip_prefix("arXiv:") {
        return !id.is_empty();
    }
    let doi = ["https://doi.org/", "http://doi.org/", "doi:"]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .unwrap_or(text);
    doi.strip_prefix("10.")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(registrant, suffix)| {
            !registrant.is_empty()
                && registrant.chars().all(|c| c.is_ascii_digit() || c == '.')
                && !suffix.is_empty()
        })
}
