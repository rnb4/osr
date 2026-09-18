//! Writing the map as a layered SVG with Inkscape layer metadata.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt::{self, Display, Formatter},
};

use quick_xml::escape::escape;

use super::{
    ATLAS_SCALE, Args, CoordsAlign, Feature, Icons,
    borders::{Border, Type},
    rivers::Course,
    roads::Road,
    symbols::SODIPODI_NS,
};
use crate::{
    hex::{Axial, Layout, Point},
    map::{Hex, Terrain},
};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const INKSCAPE_NS: &str = "http://www.inkscape.org/namespaces/inkscape";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

pub struct Scene<'a> {
    pub args: &'a Args,
    pub layout: Layout,
    pub size: Point,
    pub hexes: &'a [Hex<'a>],
    pub icons: Option<&'a Icons<'a>>,
    pub features: &'a [Feature],
    pub rivers: &'a [Course],
    pub roads: &'a [Road],
    pub borders: &'a [Border],
    /// The types the borders index into. Unlike the road classes these are read from a file, not given as arguments.
    pub border_types: &'a [Type],
}

/// A length rounded to 2 decimal places.
pub struct Num(pub f64);

impl Display for Num {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        // Adding zero turns -0 into 0.
        write!(f, "{}", (self.0 * 100.0).round() / 100.0 + 0.0)
    }
}

impl Display for Point {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{},{}", Num(self.x), Num(self.y))
    }
}

/// A line segment as SVG path data.
struct Segment(Point, Point);

impl Display for Segment {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "M{}L{}", self.0, self.1)
    }
}

/// Writes `items` with `separator` between them.
fn write_separated<T: Display>(f: &mut Formatter, items: impl IntoIterator<Item = T>, separator: &str) -> fmt::Result {
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            f.write_str(separator)?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

fn open_layer(f: &mut Formatter, label: &str, id: &str) -> fmt::Result {
    writeln!(f, r#"  <g inkscape:groupmode="layer" inkscape:label="{label}" id="{id}">"#)
}

fn close_layer(f: &mut Formatter) -> fmt::Result {
    writeln!(f, "  </g>")
}

fn write_icon_use(f: &mut Formatter, id: usize, centre: Point, size: f64) -> fmt::Result {
    let (x, y, size) = (Num(centre.x - size / 2.0), Num(centre.y - size / 2.0), Num(size));
    writeln!(f, r##"    <use href="#icon-{id}" x="{x}" y="{y}" width="{size}" height="{size}"/>"##)
}

impl Display for Scene<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let (width, height) = (Num(self.size.x), Num(self.size.y));
        writeln!(f, r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>"#)?;
        writeln!(
            f,
            r#"<svg xmlns="{SVG_NS}" xmlns:inkscape="{INKSCAPE_NS}" xmlns:sodipodi="{SODIPODI_NS}" xmlns:xlink="{XLINK_NS}" version="1.1" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
        )?;
        if let Some(background) = &self.args.grid.background {
            let fill = escape(background);
            writeln!(f, r#"  <rect x="0" y="0" width="{width}" height="{height}" fill="{fill}" id="background"/>"#)?;
        }

        self.write_terrain(f)?;
        if let Some(icons) = self.icons {
            self.write_icons(f, icons)?;
        }
        // Courses and roads run under the grid, so the hex outlines stay unbroken where one crosses them.
        self.write_rivers(f)?;
        self.write_roads(f)?;
        if !self.args.grid.no_grid {
            self.write_grid(f)?;
        }
        if let Some(offset) = self.args.atlas.atlas {
            self.write_atlas(f, offset)?;
        }
        self.write_borders(f)?;
        self.write_features(f)?;
        self.write_labels(f)?;
        if let Some(align) = self.args.coords.coords {
            self.write_coords(f, align)?;
        }
        writeln!(f, "</svg>")
    }
}

impl Scene<'_> {
    fn grid_width(&self) -> f64 {
        self.args.grid.grid_width.unwrap_or(self.layout.radius() / 40.0)
    }

    /// Flat colour per hex, grouped by terrain so a whole terrain can be selected at once. The stroke matches the fill
    /// to hide the hairline antialiasing seams that otherwise show between adjacent hexes. Half of it lies outside the
    /// hex, so it is kept to half the grid line width, which leaves that part a quarter of the line's width from the
    /// edge and well under the line.
    fn write_terrain(&self, f: &mut Formatter) -> fmt::Result {
        let seam = Num(self.grid_width() / 2.0);
        let mut groups: BTreeMap<char, (&Terrain, Vec<Axial>)> = BTreeMap::new();
        for hex in self.hexes {
            groups.entry(hex.terrain.key).or_insert_with(|| (hex.terrain, Vec::new())).1.push(hex.axial);
        }

        open_layer(f, "Terrain", "layer-terrain")?;
        for (key, (terrain, hexes)) in groups {
            let label = terrain.name.clone().unwrap_or_else(|| format!("terrain {key}"));
            let id: String = label.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
            writeln!(
                f,
                r#"    <g inkscape:label="{}" id="terrain-{}" fill="{color}" stroke="{color}" stroke-width="{seam}" stroke-linejoin="round">"#,
                escape(&label),
                id.trim_matches('-'),
                color = escape(&terrain.color),
            )?;
            for hex in hexes {
                write!(f, r#"      <polygon points=""#)?;
                write_separated(f, self.layout.corners(self.layout.centre(hex), 1.0), " ")?;
                writeln!(f, r#""/>"#)?;
            }
            writeln!(f, "    </g>")?;
        }
        close_layer(f)
    }

    /// One terrain icon per hex. Each icon's artwork is defined once in <defs> and referenced per hex, so 300 forest
    /// hexes cost one copy of the artwork plus 300 short <use> elements.
    fn write_icons(&self, f: &mut Formatter, icons: &Icons) -> fmt::Result {
        if !icons.symbols.is_empty() {
            writeln!(f, "  <defs>")?;
            for (id, symbol) in icons.symbols.iter().enumerate() {
                writeln!(
                    f,
                    r#"    <symbol id="icon-{id}" viewBox="{}">{}</symbol>"#,
                    escape(&symbol.viewbox),
                    symbol.inner
                )?;
            }
            writeln!(f, "  </defs>")?;
        }

        open_layer(f, "Icons", "layer-icons")?;
        let size = self.args.icons.icon_scale * self.layout.hex_width();
        for &(centre, id) in &icons.placed {
            write_icon_use(f, id, centre, size)?;
        }
        close_layer(f)
    }

    /// Hex outlines, drawn above the icons so grid lines stay legible over them.
    fn write_grid(&self, f: &mut Formatter) -> fmt::Result {
        let grid = &self.args.grid;
        let edges = self.layout.edges(self.hexes.iter().map(|hex| hex.axial), 1.0, |hex| self.layout.centre(hex));

        open_layer(f, "Grid", "layer-grid")?;
        write!(f, r#"    <path d=""#)?;
        write_separated(f, edges.into_iter().map(|(a, b)| Segment(a, b)), " ")?;
        writeln!(
            f,
            r#"" fill="none" stroke="{}" stroke-width="{}" stroke-opacity="{}" stroke-linecap="round" id="hex-grid"/>"#,
            escape(&grid.grid_color),
            Num(self.grid_width()),
            Num(grid.grid_opacity),
        )?;
        close_layer(f)
    }

    /// Parent atlas hexes, ATLAS_SCALE sub-hexes across, wherever one of the sub-hexes they wholly own is on the map.
    fn write_atlas(&self, f: &mut Formatter, offset: Axial) -> fmt::Result {
        let atlas = &self.args.atlas;
        let centre_of = |cell: Axial| cell * ATLAS_SCALE + offset;
        let on_map: HashSet<Axial> = self.hexes.iter().map(|hex| hex.axial).collect();

        // A map hex owned by an atlas hex lies in the lattice cell it rounds down into, or one next to it.
        let candidates: BTreeSet<Axial> = on_map
            .iter()
            .flat_map(|hex| {
                let base =
                    Axial::new((hex.q - offset.q).div_euclid(ATLAS_SCALE), (hex.r - offset.r).div_euclid(ATLAS_SCALE));
                (-1..=1).flat_map(move |dq| (-1..=1).map(move |dr| base + Axial::new(dq, dr)))
            })
            .collect();
        let owners = candidates
            .into_iter()
            .filter(|&cell| centre_of(cell).disc(ATLAS_SCALE / 2).any(|hex| on_map.contains(&hex)));
        let edges = self.layout.edges(owners, f64::from(ATLAS_SCALE), |cell| self.layout.centre(centre_of(cell)));
        let width = atlas.atlas_width.unwrap_or(self.layout.radius() / 10.0);

        open_layer(f, "Atlas", "layer-atlas")?;
        write!(f, r#"    <path d=""#)?;
        write_separated(f, edges.into_iter().map(|(a, b)| Segment(a, b)), " ")?;
        writeln!(
            f,
            r#"" fill="none" stroke="{}" stroke-width="{}" stroke-opacity="{}" stroke-linecap="round" stroke-linejoin="round" id="atlas-grid"/>"#,
            escape(&atlas.atlas_color),
            Num(width),
            Num(atlas.atlas_opacity),
        )?;
        close_layer(f)
    }

    /// Border rings, grouped by the type they are drawn in so a whole type can be picked out at once. A ring is one
    /// closed path, so a dashed type carries its dashes right round an area instead of restarting at every hex. The
    /// layer is opened even when there are none, so there is always somewhere to draw more by hand.
    fn write_borders(&self, f: &mut Formatter) -> fmt::Result {
        let scale = self.layout.hex_width();

        open_layer(f, "Borders", "layer-borders")?;
        for (kind, ty) in self.border_types.iter().enumerate() {
            let rings: Vec<&Border> = self.borders.iter().filter(|border| border.kind == kind).collect();
            if rings.is_empty() {
                continue;
            }
            let id: String = ty.name.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
            write!(
                f,
                r#"    <g fill="none" stroke="{}" stroke-width="{}" stroke-opacity="{}" stroke-linecap="round" stroke-linejoin="round""#,
                escape(&ty.color),
                Num(ty.width * scale),
                Num(self.args.borders.border_opacity),
            )?;
            if !ty.dash.is_empty() {
                write!(f, r#" stroke-dasharray=""#)?;
                write_separated(f, ty.dash.iter().map(|&length| Num(length * scale)), ",")?;
                write!(f, r#"""#)?;
            }
            writeln!(f, r#" inkscape:label="{}" id="border-{}">"#, escape(&ty.name), id.trim_matches('-'))?;

            for border in rings {
                write!(f, r#"      <path d="M"#)?;
                write_separated(f, &border.ring, "L")?;
                writeln!(f, r#"Z"/>"#)?;
            }
            writeln!(f, "    </g>")?;
        }
        close_layer(f)
    }

    /// Water courses, each a filled outline running from a source to a mouth. The layer is opened even when there are
    /// none, so there is always somewhere to draw more by hand.
    ///
    /// Courses sharing a trunk overlap along it, which one flat colour hides, so confluences need no seam of their own.
    fn write_rivers(&self, f: &mut Formatter) -> fmt::Result {
        let rivers = &self.args.rivers;
        open_layer(f, "Rivers", "layer-rivers")?;
        if !self.rivers.is_empty() {
            writeln!(
                f,
                r#"    <g fill="{}" fill-opacity="{}" stroke="none" id="rivers">"#,
                escape(&rivers.river_color),
                Num(rivers.river_opacity),
            )?;
            for course in self.rivers {
                write!(f, r#"      <path d="M"#)?;
                write_separated(f, &course.outline, "L")?;
                writeln!(f, r#"Z"/>"#)?;
            }
            writeln!(f, "    </g>")?;
        }
        close_layer(f)
    }

    /// Ways, grouped by the kind they are drawn in so a whole class can be selected at once, and stroked rather than
    /// filled: one width along a run is what lets a trail be dashed. A run is one path, so its dashes carry the length
    /// of it instead of restarting at every hex. The layer is opened even when there are none, so there is always
    /// somewhere to draw more by hand.
    fn write_roads(&self, f: &mut Formatter) -> fmt::Result {
        let roads = &self.args.roads;
        let scale = self.layout.hex_width();

        open_layer(f, "Roads", "layer-roads")?;
        for (class, kind) in roads.road_class.iter().enumerate() {
            let ways: Vec<&Road> = self.roads.iter().filter(|road| road.class == class).collect();
            if ways.is_empty() {
                continue;
            }
            let id: String = kind.name.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
            write!(
                f,
                r#"    <g fill="none" stroke="{}" stroke-width="{}" stroke-opacity="{}" stroke-linecap="round" stroke-linejoin="round""#,
                escape(&kind.color),
                Num(kind.width * scale),
                Num(roads.road_opacity),
            )?;
            if !kind.dash.is_empty() {
                write!(f, r#" stroke-dasharray=""#)?;
                write_separated(f, kind.dash.iter().map(|&length| Num(length * scale)), ",")?;
                write!(f, r#"""#)?;
            }
            writeln!(f, r#" inkscape:label="{}" id="road-{}">"#, escape(&kind.name), id.trim_matches('-'))?;

            for road in ways {
                write!(f, r#"      <path d="M{}"#, road.anchors[0])?;
                for (control, anchor) in road.controls.iter().zip(&road.anchors[1..]) {
                    write!(f, "Q{control} {anchor}")?;
                }
                writeln!(f, r#""/>"#)?;
            }
            writeln!(f, "    </g>")?;
        }
        close_layer(f)
    }

    fn feature_size(&self) -> f64 {
        self.args.icons.feature_scale * self.layout.hex_width()
    }

    fn write_features(&self, f: &mut Formatter) -> fmt::Result {
        if self.features.is_empty() {
            return Ok(());
        }
        open_layer(f, "Features", "layer-features")?;
        for feature in self.features {
            write_icon_use(f, feature.icon, feature.centre, self.feature_size())?;
        }
        close_layer(f)
    }

    fn write_labels(&self, f: &mut Formatter) -> fmt::Result {
        let labels = &self.args.labels;
        open_layer(f, "Labels", "layer-labels")?;
        writeln!(
            f,
            r#"    <g font-family="{}" font-size="{}" font-weight="{}" font-style="{}" fill="{}" text-anchor="middle">"#,
            escape(&labels.label_font),
            Num(labels.label_size * self.layout.hex_width()),
            escape(&labels.label_weight),
            escape(&labels.label_style),
            escape(&labels.label_color),
        )?;
        // Labels hang just below their feature's icon.
        let drop = self.feature_size() * 0.72;
        for feature in self.features {
            if let Some(label) = &feature.label {
                let (x, y) = (Num(feature.centre.x), Num(feature.centre.y + drop));
                writeln!(f, r#"      <text x="{x}" y="{y}">{}</text>"#, escape(label))?;
            }
        }
        writeln!(f, "    </g>")?;
        close_layer(f)
    }

    /// Coordinates go last so they sit on top of everything else.
    fn write_coords(&self, f: &mut Formatter, align: CoordsAlign) -> fmt::Result {
        let coords = &self.args.coords;
        let size = coords.coords_size * self.layout.hex_width();
        // SVG y is a text baseline, not a midline, so centred text is nudged down by part of its size.
        let baseline = match align {
            CoordsAlign::Top => -0.52 * self.layout.radius(),
            CoordsAlign::Center => 0.35 * size,
            CoordsAlign::Bottom => 0.62 * self.layout.radius(),
        };

        open_layer(f, "Coordinates", "layer-coords")?;
        writeln!(
            f,
            r#"    <g font-family="{}" font-size="{}" fill="{}" fill-opacity="{}" text-anchor="middle">"#,
            escape(&coords.coords_font),
            Num(size),
            escape(&coords.coords_color),
            Num(coords.coords_opacity),
        )?;
        for hex in self.hexes {
            let centre = self.layout.centre(hex.axial);
            writeln!(f, r#"      <text x="{}" y="{}">{}</text>"#, Num(centre.x), Num(centre.y + baseline), hex.coord)?;
        }
        writeln!(f, "    </g>")?;
        close_layer(f)
    }
}

#[cfg(test)]
mod tests {
    use super::Num;

    #[test]
    fn numbers_round_to_two_places() {
        assert_eq!(Num(100.0).to_string(), "100");
        assert_eq!(Num(559.6152).to_string(), "559.62");
        assert_eq!(Num(1.5).to_string(), "1.5");
        assert_eq!(Num(-0.004).to_string(), "0");
        assert_eq!(Num(-2.456).to_string(), "-2.46");
    }
}
