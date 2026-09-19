//! The definitions file: one `[section]` per kind of thing, saying how a map is drawn.
//!
//! A section is not a contiguous string once it has been cut out of the file, but its lines still have to be reported
//! at the numbers the file wrote them at, so [`Lines`] carries the two together.

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
