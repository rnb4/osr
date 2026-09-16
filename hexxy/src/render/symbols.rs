//! Icon SVGs, read from a directory and made safe to inline side by side in one document.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use quick_xml::{
    Reader, Writer, XmlVersion,
    events::{BytesCData, BytesStart, BytesText, Event},
};

use crate::warn;

pub const SODIPODI_NS: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";

#[derive(Debug)]
pub struct Symbol {
    pub viewbox: String,
    /// The icon's contents as markup, ready to go inside a `<symbol>`.
    pub inner: String,
}

/// Icon variants by name.
pub type Symbols = HashMap<String, Vec<Symbol>>;

/// Reads a directory of icon SVGs.
///
/// `forest.svg` and `forest-2.svg` are both variants of forest; a non-numeric suffix like `hills-forested.svg` is a
/// separate name.
pub fn load(dir: &Path) -> Result<Symbols, String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    let entries = fs::read_dir(dir).map_err(|err| format!("could not read {}: {err}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "svg"))
        .collect();
    paths.sort();

    let mut symbols = Symbols::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else { continue };
        let text = fs::read_to_string(&path).map_err(|err| format!("could not read {}: {err}", path.display()))?;
        let symbol = Symbol::parse(&text, stem).map_err(|err| format!("{}: {err}", path.display()))?;
        symbols.entry(variant_of(stem).to_string()).or_default().push(symbol);
    }

    if symbols.is_empty() {
        // Not an error: an empty or partly drawn icon set is normal while it is being made. Terrains without an icon
        // stay flat colour.
        warn(format_args!("no .svg files in {}; rendering flat colour", dir.display()));
    }
    Ok(symbols)
}

/// The name a file registers under: `forest-2` is a variant of `forest`.
fn variant_of(stem: &str) -> &str {
    match stem.rsplit_once('-') {
        Some((name, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => stem,
    }
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// Whether the element `name` is editor state, like Inkscape's zoom and window size, which means nothing once inlined.
/// `root` holds the root element's attributes, where the namespace is declared.
fn is_editor_state(name: &str, root: &[(String, String)]) -> bool {
    let uri =
        |prefix: &str| root.iter().find(|(key, _)| key.strip_prefix("xmlns:") == Some(prefix)).map(|(_, uri)| uri);
    name.split_once(':').and_then(|(prefix, _)| uri(prefix)).is_some_and(|uri| uri == SODIPODI_NS)
}

fn invalid(err: impl std::fmt::Display) -> String {
    format!("not valid XML: {err}")
}

fn attributes(element: &BytesStart) -> Result<Vec<(String, String)>, String> {
    element
        .attributes()
        .map(|attr| {
            let attr = attr.map_err(invalid)?;
            let value = attr.normalized_value(XmlVersion::Implicit1_0).map_err(invalid)?;
            Ok((attr.key.into_inner().to_string(), value.into_owned()))
        })
        .collect()
}

/// What a first pass over an icon file finds: the root element's attributes, and the ids, classes and namespace
/// prefixes used inside it.
struct Scan {
    root: Vec<(String, String)>,
    ids: HashSet<String>,
    classes: HashSet<String>,
    prefixes: HashSet<String>,
}

impl Scan {
    fn new(text: &str) -> Result<Scan, String> {
        let mut reader = Reader::from_str(text);
        let mut root: Option<Vec<(String, String)>> = None;
        let mut scan =
            Scan { root: Vec::new(), ids: HashSet::new(), classes: HashSet::new(), prefixes: HashSet::new() };
        loop {
            match reader.read_event().map_err(invalid)? {
                Event::Start(element) | Event::Empty(element) => {
                    let attrs = attributes(&element)?;
                    match &root {
                        Some(root) if is_editor_state(element.name().into_inner(), root) => {}
                        Some(_) => scan.note(element.name().into_inner(), &attrs),
                        None => root = Some(attrs),
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        scan.root = root.ok_or("no root element")?;
        Ok(scan)
    }

    fn note(&mut self, name: &str, attrs: &[(String, String)]) {
        let keys = attrs.iter().map(|(key, _)| key.as_str()).filter(|key| !key.starts_with("xmlns"));
        for qualified in std::iter::once(name).chain(keys) {
            if let Some((prefix, _)) = qualified.split_once(':') {
                self.prefixes.insert(prefix.to_string());
            }
        }
        for (key, value) in attrs {
            match key.as_str() {
                "id" if !value.is_empty() => {
                    self.ids.insert(value.clone());
                }
                "class" => self.classes.extend(value.split_whitespace().map(str::to_string)),
                _ => {}
            }
        }
    }

    fn root_attr(&self, key: &str) -> Option<&str> {
        self.root.iter().find(|(k, _)| k == key).map(|(_, value)| value.as_str()).filter(|value| !value.is_empty())
    }
}

/// Rewrites an icon's ids and classes, and every reference to them, with a prefix unique to the icon.
///
/// Inlining several SVGs into one document is where naive symbol loaders break: two files both defining id="path1",
/// or Illustrator's habit of emitting <style>.st0{...}</style>, will silently corrupt each other.
struct Renamer<'a> {
    prefix: String,
    ids: &'a HashSet<String>,
    classes: &'a HashSet<String>,
}

impl Renamer<'_> {
    fn attribute(&self, key: &str, value: String) -> String {
        match key {
            "id" if !value.is_empty() => format!("{}{value}", self.prefix),
            "class" => {
                value.split_whitespace().map(|class| format!("{}{class}", self.prefix)).collect::<Vec<_>>().join(" ")
            }
            _ if local_name(key) == "href" => match value.strip_prefix('#') {
                Some(id) if self.ids.contains(id) => format!("#{}{id}", self.prefix),
                _ => value,
            },
            _ => self.urls(&value),
        }
    }

    /// Prefixes every `url(#id)` naming an id in this icon.
    fn urls(&self, value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(at) = rest.find("url(#") {
            let (before, after) = rest.split_at(at + "url(#".len());
            out.push_str(before);
            rest = after;
            if rest.find(')').is_some_and(|end| self.ids.contains(&rest[..end])) {
                out.push_str(&self.prefix);
            }
        }
        out.push_str(rest);
        out
    }

    /// Prefixes the `.class` and `#id` selectors in a stylesheet that name this icon's classes and ids.
    fn css(&self, css: &str) -> String {
        let mut out = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(at) = rest.find(['.', '#']) {
            let names = if rest[at..].starts_with('.') { self.classes } else { self.ids };
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
            let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).unwrap_or(rest.len());
            if names.contains(&rest[..end]) {
                out.push_str(&self.prefix);
            }
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }
}

impl Symbol {
    /// Reads one icon file, prefixing its ids and classes with its file stem.
    fn parse(text: &str, stem: &str) -> Result<Symbol, String> {
        let scan = Scan::new(text)?;
        let viewbox = match (scan.root_attr("viewBox"), scan.root_attr("width"), scan.root_attr("height")) {
            (Some(viewbox), _, _) => viewbox.to_string(),
            (None, Some(width), Some(height)) => {
                fn unitless(length: &str) -> &str {
                    length.trim_end_matches(|c: char| c.is_ascii_lowercase() || c == '%')
                }
                format!("0 0 {} {}", unitless(width), unitless(height))
            }
            _ => return Err("no viewBox and no width/height, so the icon has no size to scale from".into()),
        };

        // The root element is dropped, so its namespace declarations move to the top-level elements inside it.
        let namespaces: Vec<(&str, &str)> = scan
            .root
            .iter()
            .filter_map(|(key, uri)| Some((key.strip_prefix("xmlns:")?, uri.as_str())))
            .filter(|(prefix, _)| scan.prefixes.contains(*prefix))
            .collect();
        let is_editor_state = |name: &str| is_editor_state(name, &scan.root);

        let prefix: String =
            stem.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).chain("__".chars()).collect();
        let renamer = Renamer { prefix, ids: &scan.ids, classes: &scan.classes };

        let mut reader = Reader::from_str(text);
        let mut writer = Writer::new(Vec::new());
        let mut depth = 0;
        let mut skipping_from = None;
        let mut in_style = false;
        loop {
            let event = match reader.read_event().map_err(invalid)? {
                Event::Eof => break,
                Event::Start(element) => {
                    depth += 1;
                    let name = element.name().into_inner();
                    if skipping_from.is_none() && depth > 1 && is_editor_state(name) {
                        skipping_from = Some(depth);
                    }
                    in_style = local_name(name) == "style";
                    if depth == 1 || skipping_from.is_some() {
                        continue;
                    }
                    Event::Start(rename_element(&element, &renamer, if depth == 2 { &namespaces } else { &[] })?)
                }
                Event::Empty(element) => {
                    if depth == 0 || skipping_from.is_some() || is_editor_state(element.name().into_inner()) {
                        continue;
                    }
                    Event::Empty(rename_element(&element, &renamer, if depth == 1 { &namespaces } else { &[] })?)
                }
                Event::End(element) => {
                    let inside = depth > 1 && skipping_from.is_none();
                    if skipping_from == Some(depth) {
                        skipping_from = None;
                    }
                    depth -= 1;
                    in_style = false;
                    if !inside {
                        continue;
                    }
                    Event::End(element)
                }
                _ if depth == 0 || skipping_from.is_some() => continue,
                Event::Text(text) if in_style => Event::Text(BytesText::from_escaped(renamer.css(&text))),
                Event::CData(css) if in_style => Event::CData(BytesCData::new(renamer.css(&css))),
                event @ (Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)) => event,
                _ => continue,
            };
            writer.write_event(event).map_err(|err| err.to_string())?;
        }

        let inner = String::from_utf8(writer.into_inner()).expect("icon markup is written from UTF-8 text");
        Ok(Symbol { viewbox, inner })
    }
}

/// A copy of `element` with its ids, classes and references renamed, declaring `namespaces` it does not already.
fn rename_element(
    element: &BytesStart,
    renamer: &Renamer,
    namespaces: &[(&str, &str)],
) -> Result<BytesStart<'static>, String> {
    let attrs = attributes(element)?;
    let mut renamed = BytesStart::new(element.name().into_inner().to_string());
    for (prefix, uri) in namespaces {
        let key = format!("xmlns:{prefix}");
        if !attrs.iter().any(|(existing, _)| *existing == key) {
            renamed.push_attribute((key.as_str(), *uri));
        }
    }
    for (key, value) in attrs {
        let value = renamer.attribute(&key, value);
        renamed.push_attribute((key.as_str(), value.as_str()));
    }
    Ok(renamed)
}

#[cfg(test)]
mod tests {
    use super::{Symbol, variant_of};

    #[test]
    fn numeric_suffixes_are_variants() {
        assert_eq!(variant_of("forest-2"), "forest");
        assert_eq!(variant_of("hills-forested"), "hills-forested");
        assert_eq!(variant_of("a-1-2"), "a-1");
        assert_eq!(variant_of("tower-"), "tower-");
    }

    #[test]
    fn ids_classes_and_references_are_prefixed() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
  <style>.a{fill:red} .ab{} .abc{} .a-b{} #g1{}</style>
  <g id="g1" class="a ab" clip-path="url(#c)" fill="url(#missing)"><use href="#g1"/></g>
  <clipPath id="c"/>
</svg>"##;
        let symbol = Symbol::parse(svg, "my icon").unwrap();
        assert_eq!(symbol.viewbox, "0 0 10 10");
        assert_eq!(
            symbol.inner,
            r##"
  <style>.my_icon__a{fill:red} .my_icon__ab{} .abc{} .a-b{} #my_icon__g1{}</style>
  <g id="my_icon__g1" class="my_icon__a my_icon__ab" clip-path="url(#my_icon__c)" fill="url(#missing)"><use href="#my_icon__g1"/></g>
  <clipPath id="my_icon__c"/>
"##
        );
    }

    #[test]
    fn editor_state_dropped_and_namespaces_redeclared() {
        let svg = r#"<?xml version="1.0"?>
<!-- made by hand -->
<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" width="24px" height="12">
  <sodipodi:namedview id="nv"><inkscape:page/></sodipodi:namedview>
  <g inkscape:label="A &amp; B"><text>1 &lt; 2</text></g>
</svg>"#;
        let symbol = Symbol::parse(svg, "t").unwrap();
        assert_eq!(symbol.viewbox, "0 0 24 12");
        assert_eq!(
            symbol.inner,
            "\n  \n  <g xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" inkscape:label=\"A &amp; B\">\
             <text>1 &lt; 2</text></g>\n"
        );
    }

    #[test]
    fn unusable_files_are_errors() {
        assert!(Symbol::parse("<svg><g></svg>", "t").unwrap_err().starts_with("not valid XML"));
        assert!(Symbol::parse("<svg width=\"10\"/>", "t").unwrap_err().starts_with("no viewBox"));
    }
}
