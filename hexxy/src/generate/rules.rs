//! The terrain relationships file.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fmt::Display,
    fs,
    path::Path,
};

use crate::map::{Palette, content_lines};

/// What shows up alongside a terrain when it is an atlas hex's primary terrain, and how the terrain is drawn.
#[derive(Debug)]
pub struct Relations {
    pub name: String,
    pub secondary: Vec<String>,
    pub tertiary: Vec<String>,
    pub wildcard: Vec<String>,
    /// Palette names to draw the terrain with, and their weights.
    pub variants: Vec<(String, u32)>,
    /// Variants for sub-hexes near other terrain, checked in order before rolling one of `variants`.
    pub beside: Vec<BesideRule>,
}

/// A variant for sub-hexes within `reach` steps of `target`.
#[derive(Debug, PartialEq)]
pub struct BesideRule {
    pub target: Beside,
    pub variant: String,
    pub reach: i32,
}

/// What must be near a sub-hex for a `beside` variant to apply.
#[derive(Debug, PartialEq)]
pub enum Beside {
    Terrain(String),
    /// Any terrain other than the sub-hex's own.
    Other,
}

#[derive(Debug)]
pub struct Rules {
    terrains: BTreeMap<String, Relations>,
    /// The terrain an atlas hex drawn with each `variants` name has.
    terrain_of: HashMap<String, String>,
    /// The palette character of every variant, including `beside` ones.
    keys: HashMap<String, char>,
}

impl Rules {
    pub fn read(path: &Path, palette: &Palette) -> Result<Rules, String> {
        let text = fs::read_to_string(path).map_err(|err| format!("could not read rules {}: {err}", path.display()))?;
        Rules::parse(&text, path.display(), palette)
    }

    /// Parses `[terrain]` blocks of `secondary`, `tertiary`, `wildcard`, `variants` and `beside` lines, checking every
    /// terrain they name has a block and every variant is in `palette`.
    pub fn parse(text: &str, file: impl Display, palette: &Palette) -> Result<Rules, String> {
        let mut terrains: BTreeMap<String, Relations> = BTreeMap::new();
        let mut current = None;
        for (line, n) in content_lines(text) {
            let line = line.split_once('#').map_or(line, |(before, _)| before).trim();
            if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
                let name = name.trim().to_string();
                if terrains.contains_key(&name) {
                    return Err(format!("{file}:{n}: [{name}] appears twice"));
                }
                terrains.insert(name.clone(), Relations::new(name.clone()));
                current = Some(name);
                continue;
            }
            let Some(relations) = current.as_ref().and_then(|name| terrains.get_mut(name)) else {
                return Err(format!("{file}:{n}: expected a [terrain] heading first"));
            };

            let mut fields = line.split_whitespace();
            let key = fields.next().unwrap_or_default();
            let values: Vec<&str> = fields.collect();
            let names = || values.iter().map(|value| value.to_string()).collect();
            match key {
                "secondary" => relations.secondary = names(),
                "tertiary" => relations.tertiary = names(),
                "wildcard" => relations.wildcard = names(),
                "variants" => {
                    relations.variants = parse_variants(&values).map_err(|err| format!("{file}:{n}: {err}"))?
                }
                "beside" => {
                    let (target, variant, reach) = match values[..] {
                        [target, variant] => (target, variant, "1"),
                        [target, variant, reach] => (target, variant, reach),
                        _ => return Err(format!("{file}:{n}: expected 'beside TERRAIN|other VARIANT [STEPS]'")),
                    };
                    let reach = reach
                        .parse()
                        .ok()
                        .filter(|&reach| reach >= 1)
                        .ok_or_else(|| format!("{file}:{n}: beside steps must be a whole number of at least 1"))?;
                    let target = if target == "other" { Beside::Other } else { Beside::Terrain(target.to_string()) };
                    relations.beside.push(BesideRule { target, variant: variant.to_string(), reach });
                }
                _ => {
                    return Err(format!(
                        "{file}:{n}: unknown key '{key}'; expected secondary, tertiary, wildcard, variants or beside"
                    ));
                }
            }
        }
        if terrains.is_empty() {
            return Err(format!("{file}: no [terrain] blocks"));
        }

        for relations in terrains.values_mut() {
            if relations.variants.is_empty() {
                relations.variants.push((relations.name.clone(), 1));
            }
        }

        let mut palette_keys: HashMap<Cow<str>, Vec<char>> = HashMap::new();
        for terrain in palette.values() {
            palette_keys.entry(terrain.name()).or_default().push(terrain.key);
        }

        let mut terrain_of = HashMap::new();
        let mut keys = HashMap::new();
        for relations in terrains.values() {
            let name = &relations.name;
            let targets = relations.beside.iter().filter_map(|rule| match &rule.target {
                Beside::Terrain(target) => Some(target),
                Beside::Other => None,
            });
            for other in relations.secondary.iter().chain(&relations.tertiary).chain(&relations.wildcard).chain(targets)
            {
                if !terrains.contains_key(other) {
                    return Err(format!("{file}: [{name}] refers to '{other}', which has no [{other}] block"));
                }
            }

            let drawn = relations
                .variants
                .iter()
                .map(|(variant, _)| variant)
                .chain(relations.beside.iter().map(|rule| &rule.variant));
            for variant in drawn {
                let key = match palette_keys.get(variant.as_str()).map(Vec::as_slice) {
                    Some(&[key]) => key,
                    Some(_) => {
                        return Err(format!("{file}: [{name}] variant '{variant}' names several palette entries"));
                    }
                    None => return Err(format!("{file}: [{name}] variant '{variant}' is not in the palette")),
                };
                keys.insert(variant.clone(), key);
            }

            // An atlas hex drawn with one of these has this terrain. `beside` variants only change how sub-hexes near
            // other terrain are drawn, so they may belong to another terrain too.
            for (variant, _) in &relations.variants {
                if let Some(owner) = terrain_of.insert(variant.clone(), name.clone())
                    && owner != *name
                {
                    return Err(format!("{file}: '{variant}' is a variant of both [{owner}] and [{name}]"));
                }
            }
        }

        Ok(Rules { terrains, terrain_of, keys })
    }

    /// The relations of a terrain these rules define.
    pub fn get(&self, terrain: &str) -> &Relations {
        &self.terrains[terrain]
    }

    /// The terrain whose `variants` include a palette name.
    pub fn terrain_of(&self, variant: &str) -> Option<&str> {
        self.terrain_of.get(variant).map(String::as_str)
    }

    /// The palette character a variant is drawn with.
    pub fn key_of(&self, variant: &str) -> char {
        self.keys[variant]
    }
}

impl Relations {
    fn new(name: String) -> Self {
        Self {
            name,
            secondary: Vec::new(),
            tertiary: Vec::new(),
            wildcard: Vec::new(),
            variants: Vec::new(),
            beside: Vec::new(),
        }
    }
}

/// Parses palette names, each optionally followed by its weight.
fn parse_variants(values: &[&str]) -> Result<Vec<(String, u32)>, String> {
    let mut variants: Vec<(String, Option<u32>)> = Vec::new();
    for value in values {
        match value.parse::<u32>() {
            Ok(0) => return Err("variant weights must be at least 1".into()),
            Ok(weight) => match variants.last_mut() {
                Some((_, slot @ None)) => *slot = Some(weight),
                _ => return Err(format!("weight {weight} must follow a variant name")),
            },
            Err(_) => variants.push((value.to_string(), None)),
        }
    }
    if variants.is_empty() {
        return Err("variants needs at least one palette name".into());
    }
    Ok(variants.into_iter().map(|(name, weight)| (name, weight.unwrap_or(1))).collect())
}

#[cfg(test)]
mod tests {
    use super::{Beside, BesideRule, Rules};
    use crate::map::parse_palette;

    fn parse(text: &str) -> Result<Rules, String> {
        let palette =
            parse_palette("~ #1 coastal-water\n_ #2 sea\nc #3 clear\nf #4 light-forest\nF #5 heavy-forest\n", "p")
                .unwrap();
        Rules::parse(text, "r", &palette)
    }

    const GOOD: &str = "# comment
[water]
secondary plains  # inline comment
tertiary  forest
variants  sea
beside    other coastal-water 2

[plains]
secondary forest
tertiary  water
variants  clear

[forest]
secondary plains
tertiary  plains water
wildcard  water
variants  light-forest 2  heavy-forest
";

    #[test]
    fn parses_relations_and_variants() {
        let rules = parse(GOOD).unwrap();
        let forest = rules.get("forest");
        assert_eq!(forest.tertiary, ["plains", "water"]);
        assert_eq!(forest.variants, [("light-forest".to_string(), 2), ("heavy-forest".to_string(), 1)]);
        let coast = BesideRule { target: Beside::Other, variant: "coastal-water".to_string(), reach: 2 };
        assert_eq!(rules.get("water").beside, [coast]);
        assert_eq!(rules.terrain_of("heavy-forest"), Some("forest"));
        assert_eq!(rules.terrain_of("coastal-water"), None);
        assert_eq!(rules.key_of("coastal-water"), '~');
        assert_eq!(rules.terrain_of("desert"), None);
        assert_eq!(rules.key_of("sea"), '_');
    }

    #[test]
    fn beside_variants_may_belong_to_another_terrain() {
        let rules =
            parse("[coastal-water]\nsecondary sea\n\n[sea]\nsecondary coastal-water\nbeside other coastal-water 2\n")
                .unwrap();
        assert_eq!(rules.terrain_of("coastal-water"), Some("coastal-water"));
        assert_eq!(rules.terrain_of("sea"), Some("sea"));
        assert_eq!(rules.get("sea").beside[0].variant, "coastal-water");
    }

    #[test]
    fn reports_mistakes() {
        let err = |text: &str| parse(text).unwrap_err();
        let plains = "[plains]\nsecondary plains\ntertiary plains\n";
        assert!(err("secondary plains\n").contains("r:1: expected a [terrain] heading first"));
        assert!(
            err("[plains]\nsecondary forest\ntertiary plains\nvariants clear\n").contains("'forest', which has no")
        );
        let sparse = parse("[plains]\nsecondary\nvariants clear\n").unwrap();
        assert!(sparse.get("plains").secondary.is_empty() && sparse.get("plains").tertiary.is_empty());
        assert!(err(plains).contains("variant 'plains' is not in the palette"));
        assert!(err(&format!("{plains}variants 2 clear\n")).contains("weight 2 must follow a variant name"));
        assert!(err(&format!("{plains}colour green\n")).contains("unknown key 'colour'"));
        assert!(err(&format!("{plains}variants clear\nbeside other clear 0\n")).contains("at least 1"));
        assert!(err(&format!("{plains}variants clear\n[plains]\n")).contains("r:5: [plains] appears twice"));
        assert!(
            err(&format!("{plains}variants clear\n[forest]\nsecondary plains\ntertiary plains\nvariants clear\n"))
                .contains("'clear' is a variant of both [forest] and [plains]")
        );
    }
}
