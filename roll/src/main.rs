use std::{
    collections::HashMap,
    fmt::Write,
    fs,
    io::{self, BufRead, IsTerminal},
    path::{Component, Path, PathBuf},
};

use anstyle::{AnsiColor, Style};
use clap::Parser;
use fastrand::Rng;
use tyche::{
    dice::{DieRoll, Rolled, roller::FastRand},
    expr::{Describe, Evaled, Expr, HasOpType, OpType},
};

const LONG_ABOUT: &str = "Dice Roller

Each argument is rolled as dice if it parses as a dice expression, \
otherwise it is read as a table file.

Table files have one entry per line; blank lines and lines starting with # are ignored. \
Anything inside backticks in an entry is rolled the same way: dice, or another table \
file relative to the current one, e.g. `2d4` goblins from `lairs.txt`

Numbers piped in on stdin pick table entries instead of random rolls, one per table \
argument, e.g. roll d4 | roll encounters.txt picks entry 2 when the d4 rolls 2. Lines \
that are not a whole number, like dice breakdowns, are ignored. Tables referenced \
inside entries are still rolled randomly.";

const DROPPED: Style = Style::new().dimmed().strikethrough();
const MARKER: Style = Style::new().dimmed();
const TOTAL: Style = Style::new().bold();
const ERROR: Style = AnsiColor::Red.on_default().bold();

#[derive(Parser, Debug)]
#[command(version, about, long_about = LONG_ABOUT)]
struct Args {
    /// Dice expressions or table files, e.g. "1d20 + 2" "1d8" encounters.txt
    #[arg(required = true)]
    rolls: Vec<String>,

    /// Print only totals and table entries
    #[arg(short, long)]
    quiet: bool,

    /// Always show the roll breakdown, even for a single die
    #[arg(short, long, conflicts_with = "quiet")]
    verbose: bool,

    /// Roll everything this many times [default: 1, or enough to use every piped number]
    #[arg(short = 'n', long, value_parser = clap::value_parser!(u32).range(1..))]
    count: Option<u32>,

    /// Print the sum of all dice totals at the end (with --quiet, print only the sum and table entries)
    #[arg(short, long)]
    sum: bool,

    /// Seed the RNG for repeatable rolls
    #[arg(long)]
    seed: Option<u64>,

    /// Maximum number of tables deep one roll may go
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    max_depth: u32,

    #[command(flatten)]
    color: colorchoice_clap::Color,
}

fn main() {
    let args = Args::parse();
    args.color.write_global();

    let mut tables = Tables::new(args.max_depth as usize);
    let mut rolls = Vec::with_capacity(args.rolls.len());
    for text in &args.rolls {
        match tables.resolve(text, Path::new("")) {
            Ok(roll) => rolls.push(roll),
            Err(err) => return error(err),
        }
    }

    // Piped numbers pick entries for the tables named on the command line, in order.
    let table_count = rolls.iter().filter(|roll| matches!(roll, Roll::Table(_))).count();
    let piped = if table_count > 0 && !io::stdin().is_terminal() {
        match read_piped(io::stdin().lock()) {
            Ok(piped) => piped,
            Err(err) => return error(err),
        }
    } else {
        Vec::new()
    };
    let piping = !piped.is_empty();
    let count = args.count.unwrap_or_else(|| if piping { piped.len().div_ceil(table_count) as u32 } else { 1 });
    let piped_len = piped.len();
    let mut piped = piped.into_iter();

    let mut picks = args.seed.map_or_else(Rng::new, Rng::with_seed);
    let dice = FastRand::new(picks.fork());
    let mut rngs = Rngs { picks, dice };
    let mut sum: i64 = 0;
    let mut first = true;
    for _ in 0..count {
        for roll in &rolls {
            if !args.quiet && !first {
                anstream::println!();
            }
            first = false;

            match roll {
                Roll::Dice(expr) => {
                    let evaled = match expr.eval(&mut rngs.dice) {
                        Ok(evaled) => evaled,
                        Err(err) => return error(format!("could not roll dice: {err}")),
                    };
                    let total = match evaled.calc() {
                        Ok(total) => total,
                        Err(err) => return error(format!("could not total dice: {err}")),
                    };
                    sum += i64::from(total);

                    if args.quiet {
                        if !args.sum {
                            anstream::println!("{total}");
                        }
                    } else if args.verbose || !is_single_die(&evaled) {
                        anstream::println!("{TOTAL}{total}{TOTAL:#}\n{}", render(&evaled));
                    } else {
                        anstream::println!("{TOTAL}{total}{TOTAL:#}");
                    }
                }
                Roll::Table(key) => {
                    let pick = match piped.next() {
                        Some(pick) => Some(pick),
                        None if piping => return error(format!("ran out of piped numbers after {piped_len}")),
                        None => None,
                    };
                    let outcome = match tables.roll(key, &mut rngs, pick) {
                        Ok(outcome) => outcome,
                        Err(err) => return error(err),
                    };

                    if args.quiet {
                        anstream::println!("{}", outcome.text);
                        continue;
                    }
                    anstream::println!("{TOTAL}{}{TOTAL:#}", outcome.text);
                    if !args.verbose {
                        continue;
                    }
                    // Show every table pick, with each entry's rolls indented under it.
                    for detail in &outcome.details {
                        let indent = "  ".repeat(detail.depth);
                        match &detail.kind {
                            DetailKind::Pick { name, roll, sides } => {
                                anstream::println!("{indent}{name} {MARKER}d{sides}{MARKER:#}[{roll}]");
                            }
                            DetailKind::Dice(evaled) => anstream::println!("{indent}  {}", render(evaled)),
                        }
                    }
                }
            }
        }
    }

    if args.sum {
        if args.quiet {
            anstream::println!("{sum}");
        } else {
            anstream::println!("\n{MARKER}sum:{MARKER:#} {TOTAL}{sum}{TOTAL:#}");
        }
    }
}

fn error(message: impl std::fmt::Display) {
    anstream::eprintln!("{ERROR}error:{ERROR:#} {message}");
    std::process::exit(1);
}

/// Reads every line of `input` that is a whole number, e.g. the totals printed by another roll.
fn read_piped(input: impl BufRead) -> Result<Vec<i64>, String> {
    let mut numbers = Vec::new();
    let mut any_text = false;
    for line in input.lines() {
        let line = line.map_err(|err| format!("could not read piped input: {err}"))?;
        let line = line.trim();
        any_text |= !line.is_empty();
        if let Ok(number) = line.parse() {
            numbers.push(number);
        }
    }
    if any_text && numbers.is_empty() {
        return Err("piped input has no numbers to pick table entries with".into());
    }
    Ok(numbers)
}

/// Something to roll: dice, or a loaded table (keyed by canonical path).
enum Roll {
    Dice(Expr),
    Table(PathBuf),
}

/// All tables reachable from the command line, loaded up front.
struct Tables {
    tables: HashMap<PathBuf, Table>,
    max_depth: usize,
}

/// A list of entries rolled on with equal odds.
struct Table {
    name: String,
    entries: Vec<Vec<Segment>>,
}

enum Segment {
    Text(String),
    Roll(Roll),
}

/// The random sources used while rolling.
struct Rngs {
    picks: Rng,
    dice: FastRand,
}

/// The text of a table roll, plus how it was produced.
struct Outcome<'a> {
    text: String,
    details: Vec<Detail<'a>>,
}

/// One step of a table roll. `depth` is 0 for the table rolled directly.
struct Detail<'a> {
    depth: usize,
    kind: DetailKind<'a>,
}

enum DetailKind<'a> {
    Pick { name: &'a str, roll: usize, sides: usize },
    Dice(Evaled<'a>),
}

impl Tables {
    fn new(max_depth: usize) -> Self {
        Self { tables: HashMap::new(), max_depth }
    }

    /// Parses `text` as dice, or failing that as a table file relative to `dir`,
    /// loading that table and every table it references.
    fn resolve(&mut self, text: &str, dir: &Path) -> Result<Roll, String> {
        match text.parse::<Expr>() {
            Ok(expr) => Ok(Roll::Dice(expr)),
            Err(err) => {
                let path = dir.join(text);
                if !path.is_file() {
                    return Err(format!("\"{text}\" is not dice or a table file: {err}"));
                }
                self.load(&path).map(Roll::Table)
            }
        }
    }

    fn load(&mut self, path: &Path) -> Result<PathBuf, String> {
        let name = tidy(path).display().to_string();
        let read_err = |err| format!("could not read table \"{name}\": {err}");
        let key = fs::canonicalize(path).map_err(read_err)?;
        if self.tables.contains_key(&key) {
            return Ok(key);
        }
        let text = fs::read_to_string(path).map_err(read_err)?;

        // Register before parsing entries so tables that reference each other terminate.
        self.tables.insert(key.clone(), Table { name: name.clone(), entries: Vec::new() });

        let dir = path.parent().unwrap_or(Path::new(""));
        let mut entries = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let entry = self.parse_entry(line, dir).map_err(|err| format!("{name} line {}: {err}", i + 1))?;
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(format!("table \"{name}\" has no entries"));
        }

        self.tables.get_mut(&key).expect("registered above").entries = entries;
        Ok(key)
    }

    fn parse_entry(&mut self, line: &str, dir: &Path) -> Result<Vec<Segment>, String> {
        if line.matches('`').count() % 2 == 1 {
            return Err("unclosed `".into());
        }
        let mut segments = Vec::new();
        // Splitting on backticks alternates plain text and roll references.
        for (i, part) in line.split('`').enumerate() {
            if i % 2 == 0 {
                if !part.is_empty() {
                    segments.push(Segment::Text(part.into()));
                }
            } else {
                let part = part.trim();
                if part.is_empty() {
                    return Err("empty ``".into());
                }
                segments.push(Segment::Roll(self.resolve(part, dir)?));
            }
        }
        Ok(segments)
    }

    /// Rolls on the table `key`, expanding any dice and tables in the picked entry.
    /// `pick` chooses the entry (starting at 1) instead of a random roll.
    fn roll(&self, key: &Path, rngs: &mut Rngs, pick: Option<i64>) -> Result<Outcome<'_>, String> {
        let mut outcome = Outcome { text: String::new(), details: Vec::new() };
        self.roll_into(key, 0, rngs, pick, &mut outcome)?;
        Ok(outcome)
    }

    fn roll_into<'a>(
        &'a self,
        key: &Path,
        depth: usize,
        rngs: &mut Rngs,
        pick: Option<i64>,
        out: &mut Outcome<'a>,
    ) -> Result<(), String> {
        let table = &self.tables[key];
        if depth >= self.max_depth {
            let max = self.max_depth;
            return Err(format!("\"{}\" is nested deeper than max depth {max}", table.name));
        }

        let len = table.entries.len();
        let index = match pick {
            Some(pick) => match usize::try_from(pick) {
                Ok(pick @ 1..) if pick <= len => pick - 1,
                _ => return Err(format!("{pick} is not an entry of \"{}\" (1-{len})", table.name)),
            },
            None => rngs.picks.usize(..len),
        };
        let kind = DetailKind::Pick { name: &table.name, roll: index + 1, sides: table.entries.len() };
        out.details.push(Detail { depth, kind });

        for segment in &table.entries[index] {
            match segment {
                Segment::Text(text) => out.text.push_str(text),
                Segment::Roll(Roll::Dice(expr)) => {
                    let dice_err =
                        |err: &dyn std::fmt::Display| format!("could not roll dice in \"{}\": {err}", table.name);
                    let evaled = expr.eval(&mut rngs.dice).map_err(|err| dice_err(&err))?;
                    let total = evaled.calc().map_err(|err| dice_err(&err))?;
                    let _ = write!(out.text, "{total}");
                    out.details.push(Detail { depth, kind: DetailKind::Dice(evaled) });
                }
                Segment::Roll(Roll::Table(key)) => self.roll_into(key, depth + 1, rngs, None, out)?,
            }
        }
        Ok(())
    }
}

/// Removes `.` and `dir/..` from a path for display, without touching the filesystem.
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            _ => out.push(component),
        }
    }
    out
}

fn is_single_die(evaled: &Evaled) -> bool {
    matches!(evaled, Evaled::Dice(rolled) if rolled.dice.count == 1 && rolled.dice.modifiers.is_empty())
}

fn render(evaled: &Evaled) -> String {
    match evaled {
        Evaled::Num(x) => x.to_string(),
        Evaled::Dice(rolled) => render_rolled(rolled),
        Evaled::Neg(x) if x.is_value() => format!("-{}", render(x)),
        Evaled::Neg(x) => format!("-({})", render(x)),
        Evaled::Add(a, b) => render_binary(evaled, '+', a, b),
        Evaled::Sub(a, b) => render_binary(evaled, '-', a, b),
        Evaled::Mul(a, b) => render_binary(evaled, '*', a, b),
        Evaled::DivDown(a, b) => render_binary(evaled, '/', a, b),
        Evaled::DivUp(a, b) => render_binary(evaled, '\\', a, b),
        _ => evaled.describe(None),
    }
}

fn render_binary(parent: &Evaled, op: char, a: &Evaled, b: &Evaled) -> String {
    let side = |child: &Evaled| {
        let wrap = matches!(
            (parent.op_type(), child.op_type()),
            (OpType::Additive | OpType::Unary, OpType::Multiplicative)
                | (OpType::Multiplicative | OpType::Unary, OpType::Additive)
                | (OpType::Unary, OpType::Unary)
        );
        if wrap { format!("({})", render(child)) } else { render(child) }
    };
    format!("{} {op} {}", side(a), side(b))
}

fn render_rolled(rolled: &Rolled) -> String {
    let rolls: Vec<String> = rolled.rolls.iter().map(render_roll).collect();
    format!("{}[{}]", rolled.dice, rolls.join(", "))
}

fn render_roll(roll: &DieRoll) -> String {
    let mut out = if roll.is_dropped() { format!("{DROPPED}{}{DROPPED:#}", roll.val) } else { roll.val.to_string() };
    if roll.is_changed() {
        let _ = write!(out, " {MARKER}(m){MARKER:#}");
    }
    if roll.is_dropped() {
        let _ = write!(out, " {MARKER}(d){MARKER:#}");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use fastrand::Rng;
    use tyche::{
        dice::roller::FastRand,
        expr::{Describe, Expr},
    };

    use super::{Rngs, Roll, Tables, read_piped, render, tidy};

    #[test]
    fn render_matches_describe_without_drops() {
        let expr: Expr = "-(1d4 + 2) * 3d6 - 5".parse().expect("parse failed");
        let evaled = expr.eval(&mut FastRand::default()).expect("eval failed");
        assert_eq!(render(&evaled), evaled.describe(None));
    }

    #[test]
    fn same_seed_same_rolls() {
        let expr: Expr = "4d6kh3 + 2d8".parse().expect("parse failed");
        let roll = |seed| {
            let evaled = expr.eval(&mut FastRand::with_seed(seed)).expect("eval failed");
            render(&evaled)
        };
        assert_eq!(roll(42), roll(42));
    }

    #[test]
    fn tidy_removes_dot_segments() {
        assert_eq!(tidy(Path::new("lairs/../monsters/./m.txt")), Path::new("monsters/m.txt"));
        assert_eq!(tidy(Path::new("../a/../../b.txt")), Path::new("../../b.txt"));
    }

    fn dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roll-test-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn rngs(seed: u64) -> Rngs {
        Rngs { picks: Rng::with_seed(seed), dice: FastRand::with_seed(seed) }
    }

    fn roll_text(tables: &Tables, roll: &Roll, seed: u64) -> Result<String, String> {
        let Roll::Table(key) = roll else { panic!("expected table") };
        tables.roll(key, &mut rngs(seed), None).map(|outcome| outcome.text)
    }

    #[test]
    fn skips_comments_and_expands_dice() {
        let dir = dir("dice");
        fs::write(dir.join("t.txt"), "# comment\n\n  A `1d1 + 1` b  \n").unwrap();
        let mut tables = Tables::new(10);
        let roll = tables.resolve("t.txt", &dir).unwrap();
        assert_eq!(roll_text(&tables, &roll, 1).unwrap(), "A 2 b");
    }

    #[test]
    fn nested_tables_resolve_relative_to_their_file() {
        let dir = dir("nested");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/a.txt"), "got `b.txt`").unwrap();
        fs::write(dir.join("sub/b.txt"), "B").unwrap();
        let mut tables = Tables::new(10);
        let roll = tables.resolve("sub/a.txt", &dir).unwrap();
        assert_eq!(roll_text(&tables, &roll, 1).unwrap(), "got B");
    }

    #[test]
    fn self_reference_loads_but_hits_max_depth() {
        let dir = dir("loop");
        fs::write(dir.join("a.txt"), "`a.txt`").unwrap();
        let mut tables = Tables::new(5);
        let roll = tables.resolve("a.txt", &dir).unwrap();
        let err = roll_text(&tables, &roll, 1).unwrap_err();
        assert!(err.contains("max depth 5"), "{err}");
    }

    #[test]
    fn bad_entries_report_line() {
        let dir = dir("bad");
        fs::write(dir.join("unclosed.txt"), "ok\n`1d6").unwrap();
        fs::write(dir.join("missing.txt"), "# c\n`nope.txt`").unwrap();
        fs::write(dir.join("empty.txt"), "# only a comment\n").unwrap();
        let mut tables = Tables::new(10);
        let err = |tables: &mut Tables, name| tables.resolve(name, &dir).err().expect("should fail");
        assert!(err(&mut tables, "unclosed.txt").ends_with("unclosed.txt line 2: unclosed `"));
        assert!(err(&mut tables, "missing.txt").contains("missing.txt line 2: \"nope.txt\" is not dice"));
        assert!(err(&mut tables, "empty.txt").contains("has no entries"));
    }

    #[test]
    fn same_seed_same_picks() {
        let dir = dir("seed");
        fs::write(dir.join("t.txt"), "a `1d6`\nb `1d20`\nc\nd\ne\nf").unwrap();
        let mut tables = Tables::new(10);
        let roll = tables.resolve("t.txt", &dir).unwrap();
        let picks = |seed| (0..10u64).map(|i| roll_text(&tables, &roll, seed + i).unwrap()).collect::<Vec<_>>();
        assert_eq!(picks(42), picks(42));
    }

    #[test]
    fn piped_input_keeps_only_whole_numbers() {
        let input = "2\n5\n2d4[2, 3]\n\nsum: 7\n  -1 \n";
        assert_eq!(read_piped(input.as_bytes()).unwrap(), vec![2, 5, -1]);
        assert_eq!(read_piped("".as_bytes()).unwrap(), Vec::<i64>::new());
        assert!(read_piped("wolves (3)\n".as_bytes()).unwrap_err().contains("no numbers"));
    }

    #[test]
    fn pick_chooses_entry_and_nested_tables_stay_random() {
        let dir = dir("pick");
        fs::write(dir.join("t.txt"), "a\nb `u.txt`\nc").unwrap();
        fs::write(dir.join("u.txt"), "x\ny").unwrap();
        let mut tables = Tables::new(10);
        let Roll::Table(key) = tables.resolve("t.txt", &dir).unwrap() else { panic!("expected table") };
        let pick = |pick| tables.roll(&key, &mut rngs(1), Some(pick)).map(|outcome| outcome.text);
        assert_eq!(pick(1).unwrap(), "a");
        assert!(["b x", "b y"].contains(&pick(2).unwrap().as_str()));
        assert_eq!(pick(3).unwrap(), "c");
        for bad in [0, 4, -1] {
            assert!(pick(bad).unwrap_err().contains("t.txt\" (1-3)"));
        }
    }
}
