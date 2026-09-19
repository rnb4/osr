//! The definitions file: one `[section]` per kind of thing, saying how a map is drawn.
//!
//! A section is not a contiguous string once it has been cut out of the file, but its lines still have to be reported
//! at the numbers the file wrote them at, so [`Lines`] carries the two together.

use std::{collections::BTreeMap, fmt::Display, fs, path::Path};

/// Lines that are not blank or comments (starting with # or //), with their line numbers.
pub fn content_lines(text: &str) -> impl Iterator<Item = (&str, usize)> {
    text.lines().zip(1..).filter(|(line, _)| {
        let line = line.trim();
        !(line.is_empty() || line.starts_with('#') || line.starts_with("//"))
    })
}

/// Strips a `//` comment from the rest of a line.
///
/// `#` cannot be used this way: every colour starts with one. `content_lines` already drops a line that is nothing but
/// a `//` comment, so this only has to handle one that follows a value.
pub fn strip_inline(line: &str) -> &str {
    line.split_once("//").map_or(line, |(before, _)| before).trim_end()
}

/// Content lines with the numbers they had in the file they came from.
#[derive(Clone, Debug, Default)]
pub struct Lines<'a>(Vec<(&'a str, usize)>);

impl<'a> Lines<'a> {
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, usize)> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn first(&self) -> Option<(&'a str, usize)> {
        self.0.first().copied()
    }
}

/// Whole text straight to lines, so a caller with a file in hand (and every test fixture) needs no ceremony.
impl<'a> From<&'a str> for Lines<'a> {
    fn from(text: &'a str) -> Self {
        Lines(content_lines(text).map(|(line, n)| (strip_inline(line), n)).collect())
    }
}

impl<'a> FromIterator<(&'a str, usize)> for Lines<'a> {
    fn from_iter<I: IntoIterator<Item = (&'a str, usize)>>(lines: I) -> Self {
        Lines(lines.into_iter().collect())
    }
}

/// One `[name]` section: the name as written, the line its heading was on, and its content lines.
#[derive(Clone, Debug)]
pub struct Section<'a> {
    pub name: String,
    pub line: usize,
    pub lines: Lines<'a>,
}

/// A definitions file cut into its sections, every line keeping its own number.
#[derive(Clone, Debug, Default)]
pub struct Split<'a> {
    /// Content before the first heading. Whether that is allowed is the caller's business.
    pub leading: Lines<'a>,
    /// In the order they were written, duplicates and all.
    pub sections: Vec<Section<'a>>,
}

/// A heading, if the line is one: `Some(Err(_))` for one typed wrong.
///
/// A line counts as a heading when it opens with '[', so a colour like #78c7f5 or a name holding brackets is never
/// mistaken for one, and a heading typed wrong is reported rather than fed to a grammar with no idea what to do
/// with it.
fn heading(line: &str) -> Option<Result<&str, &'static str>> {
    let rest = line.trim().strip_prefix('[')?;
    let Some((name, after)) = rest.split_once(']') else {
        return Some(Err("a section heading ends with ']'"));
    };
    let after = after.trim();
    if !(after.is_empty() || after.starts_with('#')) {
        return Some(Err("a section heading has nothing after ']' but a comment"));
    }
    match name.trim() {
        "" => Some(Err("a section heading needs a name, like [palette]")),
        name => Some(Ok(name)),
    }
}

/// Cuts `text` into its `[name]` sections.
///
/// Only a heading typed wrong fails here; what the names mean is left to the caller, which is what lets the same
/// splitter serve the definitions file and the terrain rules alike.
pub fn split(text: &str, file: impl Display) -> Result<Split<'_>, String> {
    let mut split = Split::default();
    let mut current: Option<Section> = None;

    for (line, n) in content_lines(text) {
        match heading(strip_inline(line)) {
            Some(Err(why)) => return Err(format!("{file}:{n}: {why}")),
            Some(Ok(name)) => {
                split.sections.extend(current.replace(Section {
                    name: name.to_string(),
                    line: n,
                    lines: Lines::default(),
                }));
            }
            None => {
                let line = strip_inline(line);
                match &mut current {
                    Some(section) => section.lines.0.push((line, n)),
                    None => split.leading.0.push((line, n)),
                }
            }
        }
    }
    split.sections.extend(current);
    Ok(split)
}

/// Every section name the file may use; anything else is a mistake worth naming.
const SECTIONS: &[&str] = &[
    "palette",
    "road.classes",
    "border.types",
    "grid",
    "atlas",
    "rivers",
    "roads",
    "borders",
    "icons",
    "labels",
    "coords",
];

/// The prefix a terrain block's heading carries, as in `[terrain.forest]`.
const TERRAIN: &str = "terrain.";

/// Reads a definitions file. The text comes back rather than a `Defs`, because a `Defs` borrows it.
pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("could not read definitions {}: {err}", path.display()))
}

/// The sections of one definitions file, looked up by name.
#[derive(Debug)]
pub struct Defs<'a> {
    file: String,
    named: BTreeMap<String, Section<'a>>,
    /// `[terrain.NAME]` blocks with the prefix stripped, in the order they were written.
    terrain: Vec<Section<'a>>,
}

impl<'a> Defs<'a> {
    /// Splits `text` and checks every heading names a section this tool knows.
    pub fn parse(text: &'a str, file: impl Display) -> Result<Defs<'a>, String> {
        let file = file.to_string();
        let split = split(text, &file)?;
        if let Some((_, n)) = split.leading.first() {
            return Err(format!("{file}:{n}: expected a [section] heading first"));
        }

        let mut named: BTreeMap<String, Section> = BTreeMap::new();
        let mut terrain = Vec::new();
        for section in split.sections {
            if let Some(name) = section.name.strip_prefix(TERRAIN) {
                // Terrain blocks pass straight through: the rules are what know a terrain from a typo, and they
                // report a repeated block themselves.
                terrain.push(Section { name: name.to_string(), ..section });
                continue;
            }
            if !SECTIONS.contains(&section.name.as_str()) {
                return Err(format!(
                    "{file}:{}: unknown section [{}]; expected [terrain.NAME] or one of {}",
                    section.line,
                    section.name,
                    SECTIONS.join(", ")
                ));
            }
            if let Some(first) = named.get(&section.name) {
                return Err(format!(
                    "{file}:{}: [{}] appears twice, first on line {}",
                    section.line, section.name, first.line
                ));
            }
            named.insert(section.name.clone(), section);
        }
        Ok(Defs { file, named, terrain })
    }

    /// The lines of a section that has to be there and has to hold something.
    pub fn required(&self, name: &str) -> Result<Lines<'a>, String> {
        match self.named.get(name) {
            None => Err(format!("{}: no [{name}] section", self.file)),
            Some(section) if section.lines.is_empty() => {
                Err(format!("{}:{}: [{name}] is empty", self.file, section.line))
            }
            Some(section) => Ok(section.lines.clone()),
        }
    }

    /// The lines of a section that may be left out, and may be empty.
    pub fn optional(&self, name: &str) -> Option<Lines<'a>> {
        self.named.get(name).map(|section| section.lines.clone())
    }

    /// The `[terrain.NAME]` blocks, as the rules parser wants them.
    pub fn terrain(&self) -> Split<'a> {
        Split { leading: Lines::default(), sections: self.terrain.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::{Defs, Lines, split};

    /// A section's name, the line its heading was on, and its content.
    type Shape<'a> = (String, usize, Vec<(&'a str, usize)>);

    /// Every section, for comparing at a glance.
    fn shape(text: &str) -> Vec<Shape<'_>> {
        split(text, "d").unwrap().sections.into_iter().map(|s| (s.name, s.line, s.lines.iter().collect())).collect()
    }

    #[test]
    fn sections_keep_their_lines_numbers() {
        let text = "# a comment\n[palette]\n~ #78c7f5 water\n\ng #95c763\n\n[grid]\nsize 50\n";
        assert_eq!(
            shape(text),
            [
                ("palette".to_string(), 2, vec![("~ #78c7f5 water", 3), ("g #95c763", 5)]),
                ("grid".to_string(), 7, vec![("size 50", 8)]),
            ]
        );
    }

    #[test]
    fn a_colour_is_not_a_heading() {
        // The '#' of a colour, and a name holding brackets, both have to survive.
        assert_eq!(shape("[palette]\n~ #78c7f5 water [deep]\n")[0].2, [("~ #78c7f5 water [deep]", 2)]);
    }

    #[test]
    fn comments_after_a_heading_are_allowed() {
        assert_eq!(shape("[palette] # terrain\n")[0].0, "palette");
        assert_eq!(shape("[palette] // terrain\n")[0].0, "palette");
        assert_eq!(shape("[grid]\nsize 50 // px\n")[0].2, [("size 50", 2)]);
    }

    #[test]
    fn headings_are_checked() {
        let fails = |text: &str| split(text, "d").unwrap_err();
        assert_eq!(fails("[palette\n"), "d:1: a section heading ends with ']'");
        assert_eq!(fails("[]\n"), "d:1: a section heading needs a name, like [palette]");
        assert_eq!(fails("[palette] terrain\n"), "d:1: a section heading has nothing after ']' but a comment");
    }

    #[test]
    fn content_before_the_first_heading_is_kept_apart() {
        let split = split("~ #78c7f5 water\n[grid]\n", "d").unwrap();
        assert_eq!(split.leading.iter().collect::<Vec<_>>(), [("~ #78c7f5 water", 1)]);
        assert_eq!(split.sections.len(), 1);
    }

    #[test]
    fn a_split_is_not_a_vocabulary() {
        // Bare names and repeats are the caller's business, which is what lets the terrain rules share this.
        assert_eq!(shape("[forest]\n[forest]\n").len(), 2);
    }

    #[test]
    fn defs_check_the_section_names() {
        let fails = |text: &str| Defs::parse(text, "d").unwrap_err();
        assert!(fails("[palate]\n").starts_with("d:1: unknown section [palate]; expected [terrain.NAME] or one of "));
        assert_eq!(fails("~ #78c7f5 water\n"), "d:1: expected a [section] heading first");
        assert_eq!(fails("[grid]\n[grid]\n"), "d:2: [grid] appears twice, first on line 1");
    }

    #[test]
    fn terrain_blocks_lose_their_prefix_and_keep_repeats() {
        let text = "[terrain.forest]\nsecondary hills\n[terrain.forest]\n";
        let defs = Defs::parse(text, "d").unwrap();
        let terrain = defs.terrain();
        assert_eq!(terrain.sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["forest", "forest"]);
        assert_eq!(terrain.sections[0].lines.iter().collect::<Vec<_>>(), [("secondary hills", 2)]);
    }

    #[test]
    fn required_sections_must_hold_something() {
        let defs = Defs::parse("[palette]\n~ #78c7f5 water\n[grid]\n", "d").unwrap();
        assert_eq!(defs.required("palette").unwrap().iter().count(), 1);
        assert_eq!(defs.required("grid").unwrap_err(), "d:3: [grid] is empty");
        assert_eq!(defs.required("atlas").unwrap_err(), "d: no [atlas] section");
        assert!(defs.optional("grid").is_some());
        assert!(defs.optional("atlas").is_none());
    }

    #[test]
    fn whole_text_converts_to_lines() {
        let lines: Lines = "# skip\n\nkeep me // not this\n".into();
        assert_eq!(lines.iter().collect::<Vec<_>>(), [("keep me", 3)]);
    }
}
