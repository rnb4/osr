//! Every setting the renderer draws by, resolved from three places at once.
//!
//! The built-in defaults come first, the definitions file's `[section]` blocks overlay them, and anything typed as a
//! flag overlays that: the flags win, and the file beats the defaults. The renderer reads the result and never sees
//! the command line, so how a setting was arrived at stops mattering past this module.

use std::fmt::Display;

use super::CoordsAlign;
use crate::{
    defs::Defs,
    hex::Axial,
    map::{self},
};

/// The sections holding settings, as opposed to the ones defining what is drawn.
const SETTINGS: &[&str] = &["grid", "atlas", "rivers", "roads", "borders", "icons", "labels", "coords"];

#[derive(Debug, Default)]
pub struct Settings {
    pub grid: Grid,
    pub atlas: Atlas,
    pub rivers: Rivers,
    pub roads: Roads,
    pub borders: Borders,
    pub icons: Icons,
    pub labels: Labels,
    pub coords: Coords,
}

#[derive(Debug)]
pub struct Grid {
    pub draw: bool,
    /// Foundry grid size in px.
    pub size: f64,
    pub color: String,
    /// None scales it with the grid size.
    pub width: Option<f64>,
    pub opacity: f64,
    pub background: Option<String>,
}

#[derive(Debug)]
pub struct Atlas {
    /// None draws no atlas lattice at all.
    pub offset: Option<Axial>,
    pub color: String,
    pub width: Option<f64>,
    pub opacity: f64,
}

#[derive(Debug)]
pub struct Rivers {
    pub draw: bool,
    pub color: String,
    pub width: f64,
    pub max_width: f64,
    pub wander: f64,
    pub opacity: f64,
    pub mouth: char,
}

#[derive(Debug)]
pub struct Roads {
    pub draw: bool,
    pub wander: f64,
    pub opacity: f64,
}

#[derive(Debug)]
pub struct Borders {
    pub draw: bool,
    pub opacity: f64,
}

#[derive(Debug)]
pub struct Icons {
    pub draw: bool,
    pub features: bool,
    pub scale: f64,
    pub feature_scale: f64,
}

#[derive(Debug)]
pub struct Labels {
    pub color: String,
    pub font: String,
    pub size: f64,
    pub weight: String,
    pub style: String,
}

#[derive(Debug)]
pub struct Coords {
    /// None prints no coordinates.
    pub show: Option<CoordsAlign>,
    pub font: String,
    pub size: f64,
    pub color: String,
    pub opacity: f64,
}

impl Default for Grid {
    fn default() -> Self {
        Grid { draw: true, size: 50.0, color: "#2b2b2b".into(), width: None, opacity: 1.0, background: None }
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Atlas { offset: None, color: "#241c14".into(), width: None, opacity: 0.85 }
    }
}

impl Default for Rivers {
    fn default() -> Self {
        Rivers {
            draw: true,
            color: "#78c7f5".into(),
            width: 0.07,
            max_width: 0.22,
            wander: 0.4,
            opacity: 1.0,
            mouth: 'M',
        }
    }
}

impl Default for Roads {
    fn default() -> Self {
        Roads { draw: true, wander: 0.25, opacity: 1.0 }
    }
}

impl Default for Borders {
    fn default() -> Self {
        Borders { draw: true, opacity: 1.0 }
    }
}

impl Default for Icons {
    fn default() -> Self {
        Icons { draw: true, features: true, scale: 0.66, feature_scale: 0.72 }
    }
}

impl Default for Labels {
    fn default() -> Self {
        Labels {
            color: "#1b1b1b".into(),
            font: "serif".into(),
            size: 0.26,
            weight: "normal".into(),
            style: "normal".into(),
        }
    }
}

impl Default for Coords {
    fn default() -> Self {
        Coords { show: None, font: "monospace".into(), size: 0.2, color: "#000".into(), opacity: 0.55 }
    }
}

/// What went wrong with one 'key value' line. The line and file are the caller's to add.
enum Bad {
    Key,
    Value(String),
}

/// The settings each section takes, for the message a mistyped key gets.
fn keys(section: &str) -> &'static str {
    match section {
        "grid" => "draw, size, color, width, opacity or background",
        "atlas" => "offset, color, width or opacity",
        "rivers" => "draw, color, width, max-width, wander, opacity or mouth",
        "roads" => "draw, wander or opacity",
        "borders" => "draw or opacity",
        "icons" => "draw, features, scale or feature-scale",
        "labels" => "color, font, size, weight or style",
        "coords" => "show, font, size, color or opacity",
        _ => "",
    }
}

impl Settings {
    /// Applies one 'key value' line of a settings section.
    fn set(&mut self, section: &str, key: &str, value: &str) -> Result<(), Bad> {
        let number = || value.parse::<f64>().map_err(|err| Bad::Value(format!("\"{value}\": {err}")));
        let flag = || match value {
            "true" | "yes" => Ok(true),
            "false" | "no" => Ok(false),
            _ => Err(Bad::Value(format!("expected true or false, got \"{value}\""))),
        };
        let one_char = || {
            let mut chars = value.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if !map::BLANKS.contains(&ch) => Ok(ch),
                _ => Err(Bad::Value(format!("expected one character, got \"{value}\""))),
            }
        };

        match (section, key) {
            ("grid", "draw") => self.grid.draw = flag()?,
            ("grid", "size") => self.grid.size = number()?,
            ("grid", "color") => self.grid.color = value.into(),
            ("grid", "width") => self.grid.width = Some(number()?),
            ("grid", "opacity") => self.grid.opacity = number()?,
            ("grid", "background") => self.grid.background = Some(value.into()),

            ("atlas", "offset") => self.atlas.offset = Some(super::parse_atlas_offset(value).map_err(Bad::Value)?),
            ("atlas", "color") => self.atlas.color = value.into(),
            ("atlas", "width") => self.atlas.width = Some(number()?),
            ("atlas", "opacity") => self.atlas.opacity = number()?,

            ("rivers", "draw") => self.rivers.draw = flag()?,
            ("rivers", "color") => self.rivers.color = value.into(),
            ("rivers", "width") => self.rivers.width = number()?,
            ("rivers", "max-width") => self.rivers.max_width = number()?,
            ("rivers", "wander") => self.rivers.wander = number()?,
            ("rivers", "opacity") => self.rivers.opacity = number()?,
            ("rivers", "mouth") => self.rivers.mouth = one_char()?,

            ("roads", "draw") => self.roads.draw = flag()?,
            ("roads", "wander") => self.roads.wander = number()?,
            ("roads", "opacity") => self.roads.opacity = number()?,

            ("borders", "draw") => self.borders.draw = flag()?,
            ("borders", "opacity") => self.borders.opacity = number()?,

            ("icons", "draw") => self.icons.draw = flag()?,
            ("icons", "features") => self.icons.features = flag()?,
            ("icons", "scale") => self.icons.scale = number()?,
            ("icons", "feature-scale") => self.icons.feature_scale = number()?,

            ("labels", "color") => self.labels.color = value.into(),
            ("labels", "font") => self.labels.font = value.into(),
            ("labels", "size") => self.labels.size = number()?,
            ("labels", "weight") => self.labels.weight = value.into(),
            ("labels", "style") => self.labels.style = value.into(),

            ("coords", "show") => {
                self.coords.show = match value {
                    "none" => None,
                    "top" => Some(CoordsAlign::Top),
                    "center" => Some(CoordsAlign::Center),
                    "bottom" => Some(CoordsAlign::Bottom),
                    _ => return Err(Bad::Value(format!("expected top, center, bottom or none, got \"{value}\""))),
                }
            }
            ("coords", "font") => self.coords.font = value.into(),
            ("coords", "size") => self.coords.size = number()?,
            ("coords", "color") => self.coords.color = value.into(),
            ("coords", "opacity") => self.coords.opacity = number()?,

            _ => return Err(Bad::Key),
        }
        Ok(())
    }

    /// Applies the flags, over whatever the file and the defaults left.
    ///
    /// A `--no-…` flag can only turn its layer off: clap cannot tell an absent one from a false one, so a file
    /// saying `draw false` cannot be turned back on from the command line.
    pub fn apply_flags(&mut self, args: &super::Args) {
        let (grid, atlas, rivers, roads) = (&args.grid, &args.atlas, &args.rivers, &args.roads);
        let (borders, icons, labels, coords) = (&args.borders, &args.icons, &args.labels, &args.coords);

        if let Some(size) = args.grid_size {
            self.grid.size = size;
        }
        if grid.no_grid {
            self.grid.draw = false;
        }
        if let Some(color) = &grid.grid_color {
            self.grid.color = color.clone();
        }
        if let Some(width) = grid.grid_width {
            self.grid.width = Some(width);
        }
        if let Some(opacity) = grid.grid_opacity {
            self.grid.opacity = opacity;
        }
        if let Some(background) = &grid.background {
            self.grid.background = Some(background.clone());
        }

        if let Some(offset) = atlas.atlas {
            self.atlas.offset = Some(offset);
        }
        if let Some(color) = &atlas.atlas_color {
            self.atlas.color = color.clone();
        }
        if let Some(width) = atlas.atlas_width {
            self.atlas.width = Some(width);
        }
        if let Some(opacity) = atlas.atlas_opacity {
            self.atlas.opacity = opacity;
        }

        if rivers.no_rivers {
            self.rivers.draw = false;
        }
        if let Some(color) = &rivers.river_color {
            self.rivers.color = color.clone();
        }
        if let Some(width) = rivers.river_width {
            self.rivers.width = width;
        }
        if let Some(max) = rivers.river_max_width {
            self.rivers.max_width = max;
        }
        if let Some(wander) = rivers.river_wander {
            self.rivers.wander = wander;
        }
        if let Some(opacity) = rivers.river_opacity {
            self.rivers.opacity = opacity;
        }
        if let Some(mouth) = rivers.river_mouth {
            self.rivers.mouth = mouth;
        }

        if roads.no_roads {
            self.roads.draw = false;
        }
        if let Some(wander) = roads.road_wander {
            self.roads.wander = wander;
        }
        if let Some(opacity) = roads.road_opacity {
            self.roads.opacity = opacity;
        }

        if borders.no_borders {
            self.borders.draw = false;
        }
        if let Some(opacity) = borders.border_opacity {
            self.borders.opacity = opacity;
        }

        if icons.no_symbols {
            self.icons.draw = false;
        }
        if icons.no_features {
            self.icons.features = false;
        }
        if let Some(scale) = icons.icon_scale {
            self.icons.scale = scale;
        }
        if let Some(scale) = icons.feature_scale {
            self.icons.feature_scale = scale;
        }

        if let Some(color) = &labels.label_color {
            self.labels.color = color.clone();
        }
        if let Some(font) = &labels.label_font {
            self.labels.font = font.clone();
        }
        if let Some(size) = labels.label_size {
            self.labels.size = size;
        }
        if let Some(weight) = &labels.label_weight {
            self.labels.weight = weight.clone();
        }
        if let Some(style) = &labels.label_style {
            self.labels.style = style.clone();
        }

        if let Some(show) = coords.coords {
            self.coords.show = Some(show);
        }
        if let Some(font) = &coords.coords_font {
            self.coords.font = font.clone();
        }
        if let Some(size) = coords.coords_size {
            self.coords.size = size;
        }
        if let Some(color) = &coords.coords_color {
            self.coords.color = color.clone();
        }
        if let Some(opacity) = coords.coords_opacity {
            self.coords.opacity = opacity;
        }
    }

    /// Applies the settings sections of the definitions file, over the built-in defaults.
    pub fn apply(&mut self, defs: &Defs, file: impl Display) -> Result<(), String> {
        for section in SETTINGS {
            let Some(lines) = defs.optional(section) else { continue };
            for (line, n) in lines.iter() {
                // The value is the rest of the line, not a field: a font stack has spaces in it.
                let Some((key, value)) = line.trim().split_once(char::is_whitespace) else {
                    return Err(format!("{file}:{n}: expected 'key value'"));
                };
                match self.set(section, key, value.trim()) {
                    Ok(()) => {}
                    Err(Bad::Key) => {
                        return Err(format!(
                            "{file}:{n}: [{section}] has no setting '{key}'; expected {}",
                            keys(section)
                        ));
                    }
                    Err(Bad::Value(why)) => return Err(format!("{file}:{n}: {section} {key}: {why}")),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CoordsAlign, Settings};
    use crate::defs::Defs;

    /// Settings resolved from a definitions file, defaults underneath.
    fn settings(text: &str) -> Result<Settings, String> {
        let defs = Defs::parse(text, "d")?;
        let mut settings = Settings::default();
        settings.apply(&defs, "d")?;
        Ok(settings)
    }

    #[test]
    fn defaults_stand_where_the_file_is_silent() {
        let settings = settings("[grid]\ncolor #ff0000\n").unwrap();
        assert_eq!(settings.grid.color, "#ff0000");
        assert_eq!(settings.grid.opacity, 1.0, "untouched keys keep their default");
        assert_eq!(settings.rivers.color, "#78c7f5", "untouched sections too");
        assert!(settings.grid.width.is_none(), "a default that is an absence stays absent");
    }

    #[test]
    fn every_section_round_trips() {
        let settings = settings(
            "[grid]\ndraw false\nsize 80\nwidth 1.25\nbackground #f3ead8\n\
             [atlas]\noffset 1,4\nopacity 0.5\n\
             [rivers]\nmax-width 0.3\nmouth X\n\
             [roads]\nwander 0.1\n\
             [borders]\nopacity 0.7\n\
             [icons]\nfeatures false\nfeature-scale 0.9\n\
             [labels]\nsize 0.3\n\
             [coords]\nshow bottom\n",
        )
        .unwrap();
        assert!(!settings.grid.draw);
        assert_eq!((settings.grid.size, settings.grid.width), (80.0, Some(1.25)));
        assert_eq!(settings.grid.background.as_deref(), Some("#f3ead8"));
        assert_eq!(settings.atlas.offset.map(|a| (a.q, a.r)), Some((1, 4)));
        assert_eq!((settings.rivers.max_width, settings.rivers.mouth), (0.3, 'X'));
        assert_eq!(settings.roads.wander, 0.1);
        assert_eq!(settings.borders.opacity, 0.7);
        assert!(!settings.icons.features);
        assert_eq!(settings.icons.feature_scale, 0.9);
        assert_eq!(settings.labels.size, 0.3);
        assert_eq!(settings.coords.show, Some(CoordsAlign::Bottom));
    }

    #[test]
    fn a_value_is_the_rest_of_the_line() {
        // A font stack has spaces and commas in it, so the value cannot be one whitespace-split field.
        let settings = settings("[labels]\nfont IM FELL English, Georgia, serif\n").unwrap();
        assert_eq!(settings.labels.font, "IM FELL English, Georgia, serif");
    }

    #[test]
    fn an_atlas_offset_wraps_the_way_the_flag_does() {
        let settings = settings("[atlas]\noffset -1, 7\n").unwrap();
        assert_eq!(settings.atlas.offset.map(|a| (a.q, a.r)), Some((4, 2)));
    }

    #[test]
    fn coordinates_can_be_turned_off_by_name() {
        assert!(settings("[coords]\nshow none\n").unwrap().coords.show.is_none());
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let fails = |text: &str| settings(text).unwrap_err();
        assert_eq!(
            fails("[rivers]\ncolour #fff\n"),
            "d:2: [rivers] has no setting 'colour'; expected draw, color, width, max-width, wander, opacity or mouth"
        );
        assert_eq!(fails("[rivers]\nwidth wide\n"), "d:2: rivers width: \"wide\": invalid float literal");
        assert_eq!(fails("[grid]\ndraw maybe\n"), "d:2: grid draw: expected true or false, got \"maybe\"");
        assert_eq!(fails("[grid]\nopacity\n"), "d:2: expected 'key value'");
        assert_eq!(fails("[rivers]\nmouth MM\n"), "d:2: rivers mouth: expected one character, got \"MM\"");
        assert_eq!(
            fails("[coords]\nshow middle\n"),
            "d:2: coords show: expected top, center, bottom or none, got \"middle\""
        );
    }

    #[test]
    fn definition_sections_are_not_settings() {
        // [palette] and the rest are read by their own parsers; apply must walk straight past them.
        let settings = settings("[palette]\n~ #78c7f5 water\n[grid]\nsize 60\n").unwrap();
        assert_eq!(settings.grid.size, 60.0);
    }
}
