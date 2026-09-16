//! The `render` subcommand: a layered hex map SVG for a Foundry VTT scene, from a plain text grid.

mod features;
mod rivers;
mod svg;
mod symbols;

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use clap::ValueEnum;

use crate::{
    found,
    hex::{ATLAS_SCALE, Axial, Layout, Point, Top},
    map::{self, Coord, Map},
    random::stable_rng,
    warn,
};
use features::FeatureSpec;
use rivers::{Channel, Style};
use svg::{Num, Scene};
use symbols::{Symbol, Symbols};

pub const LONG_ABOUT: &str =
    "Render a staggered text grid of terrain as a layered hex map SVG, sized for a Foundry VTT \
scene.

The map is typed staggered, so the text previews the drawing. '.' and space mean \"no hex here\".

  pointy (default)
      One line per hex row, with hexes 2 characters apart. Alternate rows are indented by one
      space.

          ~ ~ ~ ~
           ~ ~ ~ ~
          ~ ~ ~ ~

  flat
      Two lines per hex row. Even lines hold the even columns, 4 characters apart; odd lines
      hold the odd columns, indented by 2 and drawn half a hex lower.

          ~   ~   ~   ~          line 0 -> columns 0, 2, 4, 6   row 0
            ~   ~   ~   ~        line 1 -> columns 1, 3, 5, 7   row 0
          ~   ~   ~   ~          line 2 -> columns 0, 2, 4, 6   row 1

When not given, the palette and symbols directory are picked up from the current directory, and features from a \
file named after the map, e.g. swamp-coast.features.txt beside swamp-coast.txt.

Layers, in draw order: Terrain, Icons, Rivers, Roads, Grid, Atlas, Borders, Features, Labels, Coordinates. Roads and \
borders run along hex edges, so those layers are left empty for drawing by hand.

Rivers are drawn from a second staggered grid, MAP.rivers.txt beside the map, laid out the same way so the columns \
that line up in the text line up on the map. Any non-blank character is a river hex, and --river-mouth (default 'M') \
marks where a course ends: normally the sea or lake hex it empties into, so the river visibly reaches the water \
rather than stopping short. Courses run through hex centres, crossing between hexes at the midpoint of the edge they \
share, and widen with the square root of the hexes draining into them, so every tributary broadens the trunk it \
joins. Adjacent river hexes always join into one network, so two separate rivers cannot run side by side. The Rivers \
layer stays editable for drawing more by hand.

The image has no margin, so hex (0,0) sits on the scene origin. The Foundry scene settings to use are printed after \
rendering.";

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Text grid of terrain characters
    map: PathBuf,

    /// Terrain palette, '<char> <color> [name]' per line [default: palette or palette.txt]
    palette: Option<PathBuf>,

    /// Where to write the SVG
    #[arg(short, long, default_value = "map.svg")]
    output: PathBuf,

    /// Hex orientation
    #[arg(long, value_enum, default_value_t)]
    top: Top,

    /// Foundry grid size in px: the width of a pointy-top hex, or the height of a flat-top one
    #[arg(short, long, default_value_t = 50.0)]
    grid_size: f64,

    /// Picks between icon variants; the same seed always picks the same ones
    #[arg(long, default_value_t = 0)]
    seed: u64,

    #[command(flatten)]
    icons: IconArgs,

    #[command(flatten)]
    grid: GridArgs,

    #[command(flatten)]
    atlas: AtlasArgs,

    #[command(flatten)]
    rivers: RiverArgs,

    #[command(flatten)]
    labels: LabelArgs,

    #[command(flatten)]
    coords: CoordsArgs,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Icons")]
struct IconArgs {
    /// Directory of icon SVGs named after terrains and feature types; forest-2.svg is a variant of forest
    /// [default: symbols, if present]
    #[arg(long, value_name = "DIR", conflicts_with = "no_symbols")]
    symbols: Option<PathBuf>,

    /// Draw no icons, even if a symbols directory is present
    #[arg(long)]
    no_symbols: bool,

    /// Point features, '<col> <row> <type> [label]' per line; the type names an icon [default: MAP.features.txt
    /// beside the map, if present]
    #[arg(long, value_name = "FILE", conflicts_with = "no_features")]
    features: Option<PathBuf>,

    /// Draw no features, even if a features file is present
    #[arg(long)]
    no_features: bool,

    /// Terrain icon size as a fraction of hex width
    #[arg(long, default_value_t = 0.66)]
    icon_scale: f64,

    /// Feature icon size as a fraction of hex width
    #[arg(long, default_value_t = 0.72)]
    feature_scale: f64,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Grid")]
struct GridArgs {
    /// Draw no grid lines
    #[arg(long)]
    no_grid: bool,

    /// Grid line colour
    #[arg(long, default_value = "#2b2b2b")]
    grid_color: String,

    /// Grid line width in px [default: scales with --grid-size]
    #[arg(long)]
    grid_width: Option<f64>,

    /// Grid line opacity; below 1 the grid blends with the terrain under it
    #[arg(long, default_value_t = 1.0)]
    grid_opacity: f64,

    /// Colour behind the whole map
    #[arg(long)]
    background: Option<String>,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Atlas")]
struct AtlasArgs {
    /// Overlay the parent atlas hexes, 5 sub-hexes across, with the lattice starting on axial sub-hex Q,R. Values
    /// wrap at 5, so there are 25 distinct positions
    #[arg(long, value_name = "Q,R", value_parser = parse_atlas_offset, allow_hyphen_values = true)]
    atlas: Option<Axial>,

    /// Atlas line colour
    #[arg(long, default_value = "#241c14")]
    atlas_color: String,

    /// Atlas line width in px [default: scales with --grid-size]
    #[arg(long)]
    atlas_width: Option<f64>,

    /// Atlas line opacity
    #[arg(long, default_value_t = 0.85)]
    atlas_opacity: f64,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Rivers")]
struct RiverArgs {
    /// Staggered grid of river hexes, laid out like the map; any non-blank character is a river hex [default:
    /// MAP.rivers.txt beside the map, if present]
    #[arg(long, value_name = "FILE", conflicts_with = "no_rivers")]
    rivers: Option<PathBuf>,

    /// Draw no rivers, even if a rivers file is present
    #[arg(long)]
    no_rivers: bool,

    /// The character marking where a course ends, normally on the sea or lake hex it empties into
    #[arg(long, value_name = "CHAR", default_value_t = 'M')]
    river_mouth: char,

    /// River colour
    #[arg(long, default_value = "#78c7f5")]
    river_color: String,

    /// Width of a headwater as a fraction of hex width; a course widens with the square root of the hexes draining
    /// into it
    #[arg(long, default_value_t = 0.07)]
    river_width: f64,

    /// The widest a course is drawn, as a fraction of hex width
    #[arg(long, default_value_t = 0.22)]
    river_max_width: f64,

    /// How far a course strays from the straight run between hex centres, as a fraction of hex radius; 0 draws it
    /// perfectly regular. Which way it strays is picked by --seed
    #[arg(long, default_value_t = 0.4)]
    river_wander: f64,

    /// River opacity
    #[arg(long, default_value_t = 1.0)]
    river_opacity: f64,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Labels")]
struct LabelArgs {
    /// Feature label colour
    #[arg(long, default_value = "#1b1b1b")]
    label_color: String,

    /// Font family for feature labels, as a CSS stack, e.g. "IM FELL English, Georgia, serif". The font must be
    /// installed wherever the SVG is rendered
    #[arg(long, default_value = "serif")]
    label_font: String,

    /// Label size as a fraction of hex width
    #[arg(long, default_value_t = 0.26)]
    label_size: f64,

    /// normal, bold, or a number like 600
    #[arg(long, default_value = "normal")]
    label_weight: String,

    /// normal or italic
    #[arg(long, default_value = "normal")]
    label_style: String,
}

#[derive(clap::Args, Debug)]
#[command(next_help_heading = "Coordinates")]
struct CoordsArgs {
    /// Print each hex's column and row on it, at the given height in the hex
    #[arg(
        long,
        value_name = "ALIGN",
        value_enum,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "center"
    )]
    coords: Option<CoordsAlign>,

    /// Font for coordinates
    #[arg(long, default_value = "monospace")]
    coords_font: String,

    /// Coordinate size as a fraction of hex width
    #[arg(long, default_value_t = 0.2)]
    coords_size: f64,

    /// Coordinate colour
    #[arg(long, default_value = "#000")]
    coords_color: String,

    /// Coordinate opacity
    #[arg(long, default_value_t = 0.55)]
    coords_opacity: f64,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum CoordsAlign {
    Top,
    Center,
    Bottom,
}

/// Parses `--atlas Q,R`, wrapped into the lattice's distinct positions.
fn parse_atlas_offset(text: &str) -> Result<Axial, String> {
    let (q, r) = text.split_once(',').ok_or("expected Q,R, e.g. 1,4")?;
    let parse = |value: &str| value.trim().parse::<i32>().map_err(|err| format!("\"{value}\": {err}"));
    Ok(Axial::new(parse(q)?.rem_euclid(ATLAS_SCALE), parse(r)?.rem_euclid(ATLAS_SCALE)))
}

/// Icon artwork and the terrain icons placed with it.
#[derive(Default)]
struct Icons<'a> {
    /// Artwork in first-use order; an icon's id is its index.
    symbols: Vec<&'a Symbol>,
    /// Each terrain icon's centre and id.
    placed: Vec<(Point, usize)>,
    ids: HashMap<(&'a str, usize), usize>,
}

impl<'a> Icons<'a> {
    /// The id of `variant` of the icon `name`, defining it on first use.
    fn id(&mut self, name: &'a str, variants: &'a [Symbol], variant: usize) -> usize {
        *self.ids.entry((name, variant)).or_insert_with(|| {
            self.symbols.push(&variants[variant]);
            self.symbols.len() - 1
        })
    }
}

/// A feature's icon, placed on its hex, with its label.
struct Feature {
    centre: Point,
    icon: usize,
    label: Option<String>,
}

pub fn run(args: Args) -> Result<(), String> {
    let palette_path = found(args.palette.as_deref(), &["palette", "palette.txt"], Path::is_file)
        .ok_or("no palette given, and no palette or palette.txt in the current directory")?;
    let palette = map::read_palette(&palette_path)?;
    let map = map::read_map(&args.map, &palette, args.top)?;

    // Foundry sets a hexagon's centre-to-vertex size to gridSize/sqrt(3), making a pointy-top hex exactly gridSize
    // wide and a flat-top hex exactly gridSize tall. Its grid starts at the scene origin, so the map has no margin.
    let radius = args.grid_size / 3f64.sqrt();
    let (layout, size) = Layout::fit(args.top, radius, map.hexes.iter().map(|hex| hex.axial));

    let symbols_dir =
        if args.icons.no_symbols { None } else { found(args.icons.symbols.as_deref(), &["symbols"], Path::is_dir) };
    let features_path = if args.icons.no_features {
        None
    } else {
        args.icons.features.clone().or_else(|| Some(sidecar_for(&args.map, ".features.txt")).filter(|p| p.is_file()))
    };
    let rivers_path = if args.rivers.no_rivers {
        None
    } else {
        args.rivers.rivers.clone().or_else(|| Some(sidecar_for(&args.map, ".rivers.txt")).filter(|p| p.is_file()))
    };

    let symbols = symbols_dir.map(|dir| symbols::load(&dir).map(|symbols| (dir, symbols))).transpose()?;
    let specs = match &features_path {
        Some(path) => features::read_features(path, &map)?,
        None => Vec::new(),
    };
    let (icons, features) = match &symbols {
        Some((dir, symbols)) => {
            let (icons, features) = place_icons(&args, &layout, &map, &specs, symbols, dir)?;
            (Some(icons), features)
        }
        None if specs.is_empty() => (None, Vec::new()),
        None => {
            return Err("features are drawn as icons, so they need a symbols directory (or pass --no-features)".into());
        }
    };

    let courses = match &rivers_path {
        Some(path) => {
            let style = Style {
                channel: Channel {
                    headwater: args.rivers.river_width * layout.hex_width(),
                    max: args.rivers.river_max_width * layout.hex_width(),
                },
                wander: args.rivers.river_wander,
                seed: args.seed,
            };
            rivers::read_rivers(path, &map, &layout, args.top, args.rivers.river_mouth, style)?
        }
        None => Vec::new(),
    };

    let scene = Scene {
        args: &args,
        layout,
        size,
        hexes: &map.hexes,
        icons: icons.as_ref(),
        features: &features,
        rivers: &courses,
    };
    fs::write(&args.output, scene.to_string())
        .map_err(|err| format!("could not write {}: {err}", args.output.display()))?;

    print_foundry_settings(&args, &map, size);

    let mut counts: BTreeMap<Cow<str>, usize> = BTreeMap::new();
    for hex in &map.hexes {
        *counts.entry(hex.terrain.name()).or_default() += 1;
    }
    let counts: Vec<String> = counts.iter().map(|(name, count)| format!("{count} {name}")).collect();
    println!(
        "{}: {} {}-top hexes, {}x{}px ({})",
        args.output.display(),
        map.hexes.len(),
        args.top,
        Num(size.x),
        Num(size.y),
        counts.join(", ")
    );
    Ok(())
}

/// A file that goes with a map: `swamp-coast.txt` has `swamp-coast.features.txt` beside it.
fn sidecar_for(map: &Path, suffix: &str) -> PathBuf {
    let mut name = map.file_stem().unwrap_or(map.as_os_str()).to_os_string();
    name.push(suffix);
    map.with_file_name(name)
}

/// Chooses an icon for every hex not covered by a feature, and for every feature.
fn place_icons<'a>(
    args: &Args,
    layout: &Layout,
    map: &Map,
    specs: &[FeatureSpec],
    symbols: &'a Symbols,
    dir: &Path,
) -> Result<(Icons<'a>, Vec<Feature>), String> {
    let mut icons = Icons::default();

    let taken: HashSet<Coord> = specs.iter().map(|spec| spec.coord).collect();
    for hex in map.hexes.iter().filter(|hex| !taken.contains(&hex.coord)) {
        let Some((name, variants)) = symbols.get_key_value(hex.terrain.name().as_ref()) else { continue };
        let key = (u64::from(hex.coord.col as u32) << 32) | u64::from(hex.coord.row as u32);
        let id = icons.id(name, variants, variant(args.seed, key, variants.len()));
        icons.placed.push((layout.centre(hex.axial), id));
    }

    let features = specs
        .iter()
        .map(|spec| {
            let (name, variants) = symbols
                .get_key_value(spec.kind.as_str())
                .ok_or_else(|| format!("feature on line {}: no {}.svg in {}", spec.line, spec.kind, dir.display()))?;
            let id = icons.id(name, variants, variant(args.seed, !(spec.line as u64), variants.len()));
            Ok(Feature { centre: layout.centre(spec.axial), icon: id, label: spec.label.clone() })
        })
        .collect::<Result<_, String>>()?;

    Ok((icons, features))
}

/// A stable choice among `count` variants for the thing identified by `key`, so it keeps its icon between runs.
fn variant(seed: u64, key: u64, count: usize) -> usize {
    stable_rng(seed, 0, key).usize(..count)
}

/// Prints the Foundry VTT scene settings that line its grid up with the image.
fn print_foundry_settings(args: &Args, map: &Map, size: Point) {
    // Foundry numbers the first row of the image as row 1, not row 0, so a map starting on an indented row is one
    // where the ODD rows are offset. Verified against Foundry; getting this backwards puts the whole grid half a hex
    // out. Columns are assumed to follow the same rule.
    let (grid_type, line) = match args.top {
        Top::Pointy => ("Rows", "row"),
        Top::Flat => ("Columns", "column"),
    };
    let parity = if map.starts_offset { "Odd" } else { "Even" };

    println!("\nFoundry VTT scene settings");
    println!("  Grid Type          Hexagonal {grid_type}, {parity}");
    println!("  Grid Size          {}", Num(args.grid_size));
    println!("  Background offsets 0 and 0");
    println!("  Padding            0");
    println!("  Image dimensions   {} x {}", Num(size.x), Num(size.y));
    if map.starts_offset {
        println!("\n  note: the map's first {line} is offset. If the grid sits half a hex out, try 'Even' instead.");
    }
    if args.grid_size < 50.0 {
        warn("Foundry's minimum grid size is 50px; use --grid-size 50 or more");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Axial, parse_atlas_offset, sidecar_for, variant};

    #[test]
    fn sidecars_are_named_after_their_map() {
        assert_eq!(sidecar_for(Path::new("swamp-coast.txt"), ".features.txt"), Path::new("swamp-coast.features.txt"));
        assert_eq!(sidecar_for(Path::new("maps/atlas"), ".features.txt"), Path::new("maps/atlas.features.txt"));
        assert_eq!(sidecar_for(Path::new("swamp-coast.txt"), ".rivers.txt"), Path::new("swamp-coast.rivers.txt"));
    }

    #[test]
    fn atlas_offset_wraps() {
        assert_eq!(parse_atlas_offset("1,4"), Ok(Axial::new(1, 4)));
        assert_eq!(parse_atlas_offset("-1, 7"), Ok(Axial::new(4, 2)));
        assert!(parse_atlas_offset("1").is_err());
        assert!(parse_atlas_offset("1,x").is_err());
    }

    #[test]
    fn variants_are_stable_and_all_used() {
        let picks: Vec<usize> = (0..30).map(|key| variant(7, key, 3)).collect();
        assert_eq!(picks, (0..30).map(|key| variant(7, key, 3)).collect::<Vec<_>>());
        assert!((0..3).all(|v| picks.contains(&v)), "{picks:?}");
        assert_ne!(picks, (0..30).map(|key| variant(8, key, 3)).collect::<Vec<_>>());
    }
}
