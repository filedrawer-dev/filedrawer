//! Splits an entry into front matter and body, and reads the front matter's fields
//! (docs/entry-format.md, Front matter).

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, Scanner, TokenType};
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

/// Whether a top-level field is the server's to set: `label`, and names starting `fd_`.
fn is_reserved(name: &str) -> bool {
    name == "label" || name.starts_with("fd_")
}

/// `text` without its reserved front-matter fields (see `stored_text`).
pub(crate) fn without_reserved_fields(text: &str) -> String {
    without_fields(text, is_reserved)
}

/// `text` without the top-level front-matter fields `cut` names. Each goes with every
/// line up to the next top-level key. Text whose front matter doesn't parse, or where a
/// field to cut shares its line with another key, comes back unchanged.
pub(crate) fn without_fields(text: &str, cut: impl Fn(&str) -> bool) -> String {
    let (bom, rest) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let lines: Vec<&str> = rest.split_inclusive('\n').collect();
    let close = match lines.first() {
        Some(first) if first.trim_end() == "---" => lines
            .iter()
            .skip(1)
            .position(|line| line.trim_end() == "---")
            .map(|at| at + 1),
        _ => None,
    };
    let Some(close) = close else {
        return text.to_string();
    };
    let yaml = lines[1..close].concat();
    if uses_aliases(&yaml) {
        return text.to_string();
    }
    let mut keys = TopLevelKeys::default();
    if Parser::new_from_str(&yaml).load(&mut keys, false).is_err() {
        return text.to_string();
    }
    // A reserved key sharing its line with another key, as in a `{...}` front matter,
    // can't be cut by lines. Leave the text whole; the analyzer then warns that the
    // field was stripped, and the server refuses the entry.
    let shares_a_line = keys.keys.iter().any(|(name, line)| {
        cut(name) && keys.keys.iter().filter(|(_, other)| other == line).count() > 1
    });
    if shares_a_line {
        return text.to_string();
    }
    // Lines of the YAML (0-based) to drop; line i of the YAML is lines[i + 1].
    let mut dropped = vec![false; close - 1];
    for (index, (name, start)) in keys.keys.iter().enumerate() {
        if cut(name) {
            let end = keys
                .keys
                .get(index + 1)
                .map_or(close - 1, |(_, next)| *next);
            for line in dropped.iter_mut().take(end).skip(*start) {
                *line = true;
            }
        }
    }
    let mut stored = String::with_capacity(text.len());
    stored.push_str(bom);
    for (index, line) in lines.iter().enumerate() {
        let in_yaml = (1..close).contains(&index);
        if !(in_yaml && dropped[index - 1]) {
            stored.push_str(line);
        }
    }
    stored
}

/// Collects the top-level mapping's scalar keys with the 0-based line each starts on.
#[derive(Default)]
struct TopLevelKeys {
    depth: usize,
    expect_key: bool,
    keys: Vec<(String, usize)>,
}

impl MarkedEventReceiver for TopLevelKeys {
    fn on_event(&mut self, event: Event, mark: Marker) {
        let at_top = self.depth == 1;
        let starts_node = matches!(
            event,
            Event::Scalar(..)
                | Event::Alias(_)
                | Event::MappingStart(..)
                | Event::SequenceStart(..)
        );
        if at_top && starts_node {
            if self.expect_key
                && let Event::Scalar(name, ..) = &event
            {
                self.keys
                    .push((name.clone(), mark.line().saturating_sub(1)));
            }
            self.expect_key = !self.expect_key;
        }
        match event {
            Event::MappingStart(..) | Event::SequenceStart(..) => {
                self.depth += 1;
                if self.depth == 1 {
                    self.expect_key = true;
                }
            }
            Event::MappingEnd | Event::SequenceEnd => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
    }
}

/// Whether the YAML refers to an anchor. The loader expands aliases, so a few hundred
/// bytes of nested aliases would expand to gigabytes; the scanner alone doesn't.
pub(crate) fn uses_aliases(yaml: &str) -> bool {
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
            "embargo_until" => self.fields.embargo_until = self.embargo(value),
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

    /// `embargo_until`, which must be a YYYY-MM-DD date. Anything else is refused
    /// rather than read as "no embargo", since an early release can't be undone.
    fn embargo(&mut self, value: &Yaml) -> Option<String> {
        let written = match value {
            Yaml::Null => return None,
            Yaml::Array(_) => "a list".to_string(),
            Yaml::Hash(_) => "a mapping".to_string(),
            scalar => scalar_text(scalar).unwrap_or_default(),
        };
        if is_date(written.trim()) {
            return Some(written.trim().to_string());
        }
        self.refusals.push(Refusal::EmbargoUnreadable(written));
        None
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

/// Whether `text` is a real calendar date written YYYY-MM-DD.
fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let digits = |part: &str, length: usize| {
        part.len() == length && part.bytes().all(|b| b.is_ascii_digit())
    };
    if !(digits(year, 4) && digits(month, 2) && digits(day, 2)) {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
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
