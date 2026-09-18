//! The spec a stroked network's kinds are named by, shared by the roads and the borders.

use crate::map;

/// One kind of stroked line, named by the character standing for it on a grid.
#[derive(Clone, Debug)]
pub struct Kind {
    pub key: char,
    pub name: String,
    pub color: String,
    /// As a fraction of hex width.
    pub width: f64,
    /// Dash and gap lengths, as fractions of hex width. Empty draws a solid line.
    pub dash: Vec<f64>,
}

/// Parses one kind, '<char> <color> <width> <dash> [name]'.
///
/// The grammar follows the palette's, one line's worth at a time. `what` names the kind in the errors and `example` is
/// the spec they suggest, so a borders file is never told about road classes.
pub fn parse_kind(text: &str, what: &str, example: &str) -> Result<Kind, String> {
    let mut fields = text.split_whitespace();
    let (Some(key), Some(color), Some(width), Some(dash)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(format!("expected '<char> <color> <width> <dash> [name]', e.g. \"{example}\""));
    };

    let mut chars = key.chars();
    let (Some(key), None) = (chars.next(), chars.next()) else {
        return Err(format!("{what} key must be one character, got \"{key}\""));
    };
    if map::BLANKS.contains(&key) {
        return Err(format!("'{key}' means \"no hex here\" on a grid, so it cannot name a {what}"));
    }

    let width = width.parse::<f64>().map_err(|err| format!("width \"{width}\": {err}"))?;
    if !(width.is_finite() && width > 0.0) {
        return Err(format!("width must be a number more than zero, got {width}"));
    }

    let dash = match dash {
        "none" => Vec::new(),
        text => text
            .split(',')
            .map(|length| {
                let length = length.parse::<f64>().map_err(|err| format!("dash \"{length}\": {err}"))?;
                match length >= 0.0 {
                    true => Ok(length),
                    false => Err(format!("dash lengths cannot be negative, got {length}")),
                }
            })
            .collect::<Result<Vec<f64>, String>>()?,
    };

    let name = fields.collect::<Vec<_>>().join(" ");
    let name = if name.is_empty() { key.to_string() } else { name };
    Ok(Kind { key, name, color: color.to_string(), width, dash })
}
