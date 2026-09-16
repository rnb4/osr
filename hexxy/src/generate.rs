//! The `generate` subcommand: an atlas map expanded into sub-hexes, following Welsh Piper's Hex-based Campaign Design.

mod fill;
mod rules;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    found,
    hex::{ATLAS_SCALE, Axial, Top},
    map,
};
use rules::Rules;

pub const LONG_ABOUT: &str = "Expand an atlas map into sub-hexes, following Welsh Piper's Hex-based Campaign Design \
(Part 1).

Each character on the atlas map is an atlas hex 25 miles across, drawn in the staggered format render reads. Its \
terrain is the atlas hex's primary terrain, and generate fills it with 5-mile sub-hexes:

  centre              primary terrain
  18 whole sub-hexes  9 primary, 6 secondary, 3 tertiary (each 1 in 3 a wildcard instead)
  12 half-hexes       shared with the neighbouring atlas hexes, rolled from the primary,
                      secondary and tertiary terrain of every atlas hex sharing them

Secondary, tertiary and wildcard terrain come from the rules file, which also names the palette variants each terrain \
is drawn with. Secondary and tertiary terrain cluster toward neighbouring atlas hexes of that terrain.

Rolls are seeded per atlas hex, so changing one atlas hex only changes its own sub-hexes and those of the atlas hexes \
next to it, whose clusters and shared half-hexes depend on it.";

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Atlas map: one character per atlas hex
    atlas: PathBuf,

    /// Where to write the sub-hex map
    #[arg(short, long, default_value = "subhex.txt")]
    output: PathBuf,

    /// Palette naming each map character [default: palette or palette.txt]
    #[arg(long, value_name = "FILE")]
    palette: Option<PathBuf>,

    /// Terrain relationships and variants
    #[arg(long, value_name = "FILE", default_value = "terrain.txt")]
    rules: PathBuf,

    /// Hex orientation of both maps
    #[arg(long, value_enum, default_value_t)]
    top: Top,

    /// Seeds the rolls; the same seed and atlas always give the same sub-hexes
    #[arg(long, default_value_t = 0)]
    seed: u64,
}

pub fn run(args: Args) -> Result<(), String> {
    let palette_path = found(args.palette.as_deref(), &["palette", "palette.txt"], Path::is_file)
        .ok_or("no palette given, and no palette or palette.txt in the current directory")?;
    let palette = map::read_palette(&palette_path)?;
    let rules = Rules::read(&args.rules, &palette)?;
    let atlas = map::read_map(&args.atlas, &palette, args.top)?;

    // Each atlas hex's primary terrain, and the variant drawn for it, which its centre sub-hex keeps where it can.
    let mut primaries = BTreeMap::new();
    let mut drawn = HashMap::new();
    for hex in &atlas.hexes {
        let name = hex.terrain.name();
        let terrain = rules.terrain_of(&name).ok_or_else(|| {
            format!("atlas hex {} is {name}, which no terrain in {} has as a variant", hex.coord, args.rules.display())
        })?;
        primaries.insert(hex.axial, terrain);
        drawn.insert(fill::centre(hex.axial), name.into_owned());
    }

    let (terrain, shared) = fill::terrain(&primaries, &rules, args.seed);
    let variants = fill::variants(&terrain, &drawn, &rules, args.seed);

    let cells: Vec<(Axial, char)> = variants.iter().map(|(&cell, variant)| (cell, rules.key_of(variant))).collect();
    let (text, shift) = map::write_map(&cells, args.top);
    fs::write(&args.output, text).map_err(|err| format!("could not write {}: {err}", args.output.display()))?;

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for variant in variants.values() {
        *counts.entry(variant).or_default() += 1;
    }
    let counts: Vec<String> = counts.iter().map(|(name, count)| format!("{count} {name}")).collect();
    println!(
        "{}: {} atlas hexes -> {} sub-hexes, {shared} shared along atlas edges",
        args.output.display(),
        primaries.len(),
        cells.len()
    );
    println!("  {}", counts.join(", "));

    // Atlas hex `a` is centred on sub-hex `a * ATLAS_SCALE`, which the written map moved by `shift`.
    let (q, r) = (shift.q.rem_euclid(ATLAS_SCALE), shift.r.rem_euclid(ATLAS_SCALE));
    let top = if args.top == Top::Flat { " --top flat" } else { "" };
    println!(
        "\nTo draw it with the atlas grid lined up:\n  hexxy render {}{top} --atlas {q},{r}",
        args.output.display()
    );
    Ok(())
}
