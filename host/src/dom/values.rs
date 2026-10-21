//! CSS values, read from tokens once `var()` has been put in: lengths (with
//! `calc()`), numbers and colours. What a property does with them is
//! `style.rs`'s business.

use super::css::Token;

/// How deep `calc()` may nest its brackets.
const MAX_CALC_NESTING: usize = 16;

/// The largest length, in pixels or percent, a value may come to: past it,
/// layout's arithmetic runs into infinities, and nothing on a screen needs it.
pub const MAX_LENGTH: f32 = 1_000_000.0;

fn bounded(v: f32) -> f32 {
    v.clamp(-MAX_LENGTH, MAX_LENGTH)
}

/// A length as layout needs it: absolute units are already pixels, relative
/// ones (`em`, `rem`, `vw`, …) already resolved; only a percentage — of a
/// containing block not yet known — and `auto` are left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Auto,
    Px(f32),
    /// 0–100, as written.
    Percent(f32),
    /// `calc()` with a percentage in it: `px + percent% of the basis`.
    Calc {
        px: f32,
        percent: f32,
    },
}

impl Length {
    pub const ZERO: Length = Length::Px(0.0);

    pub fn css(&self) -> String {
        use super::css::number;
        match self {
            Length::Auto => "auto".into(),
            Length::Px(p) => format!("{}px", number(*p)),
            Length::Percent(p) => format!("{}%", number(*p)),
            Length::Calc { px, percent } => {
                format!("calc({}% + {}px)", number(*percent), number(*px))
            }
        }
    }

    /// Pixels, against `basis` for a percentage; `None` for `auto`.
    pub fn resolve(&self, basis: f32) -> Option<f32> {
        match self {
            Length::Auto => None,
            Length::Px(p) => Some(*p),
            Length::Percent(p) => Some(basis * p / 100.0),
            Length::Calc { px, percent } => Some(px + basis * percent / 100.0),
        }
    }
}

/// What relative units are relative to, for the element being styled.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// The font size `em` means: the element's own, or for `font-size`
    /// itself, its parent's.
    pub em: f32,
    pub rem: f32,
    pub vw: f32,
    pub vh: f32,
}

/// A colour, RGBA, not premultiplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);

    pub fn rgb(rgb: u32) -> Color {
        Color([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255])
    }

    pub fn is_transparent(&self) -> bool {
        self.0[3] == 0
    }

    pub fn css(&self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// This colour with its alpha scaled by `f`.
    pub fn faded(self, f: f32) -> Color {
        let [r, g, b, a] = self.0;
        Color([r, g, b, (a as f32 * f.clamp(0.0, 1.0)).round() as u8])
    }
}

/// The values a property was given, split where spaces separate them
/// outside brackets: `1px solid red` is three.
pub fn words(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Function(_) | Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth = depth.saturating_sub(1),
            _ => {}
        }
        let at_space = *t == Token::Space && depth == 0;
        if at_space {
            if let Some(s) = start.take() {
                out.push(&tokens[s..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(&tokens[s..]);
    }
    out
}

/// One keyword, lowercase.
pub fn keyword(tokens: &[Token]) -> Option<String> {
    match tokens {
        [Token::Ident(s)] => Some(s.to_ascii_lowercase()),
        _ => None,
    }
}

pub fn number(tokens: &[Token]) -> Option<f32> {
    match tokens {
        [Token::Number(n)] => Some(*n),
        [Token::Function(f), rest @ ..] if f == "calc" => match calc(rest, &Ctx::NONE)? {
            Value::Num(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}

impl Ctx {
    /// For values with no relative units in them.
    pub const NONE: Ctx = Ctx {
        em: 16.0,
        rem: 16.0,
        vw: 0.0,
        vh: 0.0,
    };
}

/// A length, a percentage, or `auto`.
pub fn length(tokens: &[Token], cx: &Ctx) -> Option<Length> {
    match tokens {
        [Token::Ident(s)] if s.eq_ignore_ascii_case("auto") => Some(Length::Auto),
        _ => length_percent(tokens, cx),
    }
}

/// A length or a percentage, not `auto`.
pub fn length_percent(tokens: &[Token], cx: &Ctx) -> Option<Length> {
    match tokens {
        [Token::Percentage(p)] => Some(Length::Percent(bounded(*p))),
        [Token::Function(f), rest @ ..]
            if matches!(f.as_str(), "calc" | "min" | "max" | "clamp") =>
        {
            match calc_function(f, rest, cx)? {
                Value::Lin { px, percent } if !px.is_finite() || !percent.is_finite() => None,
                Value::Lin { px, percent: 0.0 } => Some(Length::Px(bounded(px))),
                Value::Lin { px: 0.0, percent } => Some(Length::Percent(bounded(percent))),
                Value::Lin { px, percent } => Some(Length::Calc {
                    px: bounded(px),
                    percent: bounded(percent),
                }),
                Value::Num(0.0) => Some(Length::ZERO),
                Value::Num(_) => None,
            }
        }
        _ => px(tokens, cx).map(Length::Px),
    }
}

/// An absolute length, in pixels: no percentage, no `auto`.
pub fn px(tokens: &[Token], cx: &Ctx) -> Option<f32> {
    match tokens {
        [Token::Number(n)] if *n == 0.0 => Some(0.0),
        [Token::Dimension(n, unit)] => unit_px(*n, unit, cx),
        [Token::Function(f), rest @ ..]
            if matches!(f.as_str(), "calc" | "min" | "max" | "clamp") =>
        {
            match calc_function(f, rest, cx)? {
                Value::Lin { px, percent: 0.0 } if px.is_finite() => Some(bounded(px)),
                Value::Num(0.0) => Some(0.0),
                _ => None,
            }
        }
        _ => None,
    }
}

fn unit_px(n: f32, unit: &str, cx: &Ctx) -> Option<f32> {
    let v = match unit {
        "px" => n,
        "em" => n * cx.em,
        "rem" => n * cx.rem,
        "vw" => n * cx.vw / 100.0,
        "vh" => n * cx.vh / 100.0,
        "vmin" => n * cx.vw.min(cx.vh) / 100.0,
        "vmax" => n * cx.vw.max(cx.vh) / 100.0,
        "pt" => n * 96.0 / 72.0,
        "pc" => n * 16.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "q" => n * 96.0 / 101.6,
        "ch" | "ex" => n * cx.em * 0.5,
        _ => return None,
    };
    v.is_finite().then(|| bounded(v))
}

/// What `calc()` works with: a length as pixels plus a percentage, or a plain
/// number (for `*` and `/`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Lin { px: f32, percent: f32 },
    Num(f32),
}

/// `calc(…)`, `min(…)`, `max(…)` or `clamp(…)`, from just after the name to
/// its closing bracket.
fn calc_function(name: &str, rest: &[Token], cx: &Ctx) -> Option<Value> {
    let inner = match rest.split_last() {
        Some((Token::RParen, inner)) => inner,
        _ => return None,
    };
    if name == "calc" {
        return calc(rest, cx);
    }
    // min, max and clamp: only of values that are all pixels, or all
    // numbers — a percentage cannot be compared before layout.
    let args: Vec<Value> = super::css::split_top(inner, &Token::Comma)
        .iter()
        .map(|a| expression(super::css::trim(a), cx, 0))
        .collect::<Option<_>>()?;
    let plain: Vec<f32> = args
        .iter()
        .map(|v| match v {
            Value::Lin { px, percent } if *percent == 0.0 => Some(*px),
            Value::Num(n) => Some(*n),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let lengths = matches!(args.first(), Some(Value::Lin { .. }));
    let pick = match (name, plain.as_slice()) {
        ("min", [first, ..]) => plain.iter().copied().fold(*first, f32::min),
        ("max", [first, ..]) => plain.iter().copied().fold(*first, f32::max),
        ("clamp", [lo, v, hi]) => v.max(*lo).min(*hi),
        _ => return None,
    };
    Some(if lengths {
        Value::Lin {
            px: pick,
            percent: 0.0,
        }
    } else {
        Value::Num(pick)
    })
}

/// The tokens after `calc(`, closing bracket included.
fn calc(rest: &[Token], cx: &Ctx) -> Option<Value> {
    match rest.split_last() {
        Some((Token::RParen, inner)) => expression(super::css::trim(inner), cx, 0),
        _ => None,
    }
}

/// `a + b - c`, each term `x * y / z`.
fn expression(tokens: &[Token], cx: &Ctx, depth: usize) -> Option<Value> {
    if depth > MAX_CALC_NESTING || tokens.is_empty() {
        return None;
    }
    // Split into terms at top-level + and -, keeping each one's sign. A
    // signed number straight after a term (`100%-32px`) adds itself.
    let mut terms: Vec<(f32, Vec<Token>)> = Vec::new();
    let mut sign = 1.0;
    let mut cur: Vec<Token> = Vec::new();
    let mut nest = 0usize;
    for t in tokens {
        match t {
            Token::Function(_) | Token::LParen => nest += 1,
            Token::RParen => nest = nest.saturating_sub(1),
            _ => {}
        }
        let op = match t {
            Token::Delim('+') if nest == 0 => Some(1.0),
            Token::Delim('-') if nest == 0 => Some(-1.0),
            _ => None,
        };
        if let Some(s) = op {
            if !super::css::trim(&cur).is_empty() {
                terms.push((sign, std::mem::take(&mut cur)));
            }
            sign = s;
            continue;
        }
        cur.push(t.clone());
    }
    if !super::css::trim(&cur).is_empty() {
        terms.push((sign, cur));
    }
    let mut total: Option<Value> = None;
    for (sign, term) in terms {
        let v = scale(product(super::css::trim(&term), cx, depth)?, sign);
        total = Some(match total {
            None => v,
            Some(acc) => add(acc, v)?,
        });
    }
    total
}

fn product(tokens: &[Token], cx: &Ctx, depth: usize) -> Option<Value> {
    let mut acc: Option<Value> = None;
    let mut op = '*';
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        match t {
            Token::Space => {
                i += 1;
                continue;
            }
            Token::Delim(c @ ('*' | '/')) => {
                op = *c;
                i += 1;
                continue;
            }
            _ => {}
        }
        // One factor: a value, or a bracketed expression.
        let (v, used) = match t {
            Token::LParen | Token::Function(_) => {
                let end = close(tokens, i)?;
                let inner = &tokens[i + 1..end];
                let v = match t {
                    Token::Function(f) if f == "calc" || f == "var" => {
                        expression(super::css::trim(inner), cx, depth + 1)?
                    }
                    Token::Function(f) => calc_function(f, &tokens[i + 1..=end], cx)?,
                    _ => expression(super::css::trim(inner), cx, depth + 1)?,
                };
                (v, end + 1 - i)
            }
            Token::Number(n) => (Value::Num(*n), 1),
            Token::Percentage(p) => (
                Value::Lin {
                    px: 0.0,
                    percent: *p,
                },
                1,
            ),
            Token::Dimension(n, u) => (
                Value::Lin {
                    px: unit_px(*n, u, cx)?,
                    percent: 0.0,
                },
                1,
            ),
            _ => return None,
        };
        i += used;
        acc = Some(match acc {
            None => v,
            Some(a) => match (op, a, v) {
                ('*', Value::Num(x), other) | ('*', other, Value::Num(x)) => scale(other, x),
                ('/', a, Value::Num(x)) if x != 0.0 => scale(a, 1.0 / x),
                _ => return None,
            },
        });
    }
    acc
}

/// Where the bracket opened at `start` closes.
fn close(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(start) {
        match t {
            Token::Function(_) | Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn scale(v: Value, f: f32) -> Value {
    match v {
        Value::Lin { px, percent } => Value::Lin {
            px: px * f,
            percent: percent * f,
        },
        Value::Num(n) => Value::Num(n * f),
    }
}

fn add(a: Value, b: Value) -> Option<Value> {
    match (a, b) {
        (
            Value::Lin {
                px: p1,
                percent: c1,
            },
            Value::Lin {
                px: p2,
                percent: c2,
            },
        ) => Some(Value::Lin {
            px: p1 + p2,
            percent: c1 + c2,
        }),
        (Value::Num(x), Value::Num(y)) => Some(Value::Num(x + y)),
        // `0 + 10px`: a bare zero is a length too.
        (Value::Num(z), l @ Value::Lin { .. }) | (l @ Value::Lin { .. }, Value::Num(z))
            if z == 0.0 =>
        {
            Some(l)
        }
        _ => None,
    }
}

/// A colour: `#rgb`, `#rrggbbaa`, `rgb()`, `hsl()`, a name, `transparent`,
/// or `currentcolor`, which is `current`.
pub fn color(tokens: &[Token], current: Color) -> Option<Color> {
    match tokens {
        [Token::Hash(hex)] => hex_color(hex),
        [Token::Ident(name)] => {
            let name = name.to_ascii_lowercase();
            match name.as_str() {
                "transparent" => Some(Color::TRANSPARENT),
                "currentcolor" => Some(current),
                _ => named(&name).map(Color::rgb),
            }
        }
        [Token::Function(f), rest @ ..] => {
            let inner = match rest.split_last() {
                Some((Token::RParen, inner)) => inner,
                _ => return None,
            };
            match f.as_str() {
                "rgb" | "rgba" => rgb_function(inner),
                "hsl" | "hsla" => hsl_function(inner),
                _ => None,
            }
        }
        _ => None,
    }
}

fn hex_color(hex: &str) -> Option<Color> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|d| d * 17);
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(match hex.len() {
        3 => Color([digit(0)?, digit(1)?, digit(2)?, 255]),
        4 => Color([digit(0)?, digit(1)?, digit(2)?, digit(3)?]),
        6 => Color([pair(0)?, pair(2)?, pair(4)?, 255]),
        8 => Color([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
        _ => return None,
    })
}

/// The arguments of `rgb()` or `hsl()`, comma- or space-separated, with an
/// optional `/ alpha`.
fn channels(inner: &[Token]) -> Option<(Vec<Token>, Option<Token>)> {
    let flat: Vec<Token> = inner
        .iter()
        .filter(|t| !matches!(t, Token::Space | Token::Comma))
        .cloned()
        .collect();
    match flat.iter().position(|t| *t == Token::Delim('/')) {
        Some(at) if at == 3 && flat.len() == 5 => Some((flat[..3].to_vec(), Some(flat[4].clone()))),
        Some(_) => None,
        None if flat.len() == 4 => Some((flat[..3].to_vec(), Some(flat[3].clone()))),
        None if flat.len() == 3 => Some((flat, None)),
        None => None,
    }
}

fn alpha(t: Option<Token>) -> Option<u8> {
    let a = match t {
        None => 1.0,
        Some(Token::Number(n)) => n,
        Some(Token::Percentage(p)) => p / 100.0,
        _ => return None,
    };
    Some((a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn rgb_function(inner: &[Token]) -> Option<Color> {
    let (c, a) = channels(inner)?;
    let channel = |t: &Token| -> Option<u8> {
        let v = match t {
            Token::Number(n) => *n,
            Token::Percentage(p) => p * 2.55,
            _ => return None,
        };
        Some(v.clamp(0.0, 255.0).round() as u8)
    };
    Some(Color([
        channel(&c[0])?,
        channel(&c[1])?,
        channel(&c[2])?,
        alpha(a)?,
    ]))
}

fn hsl_function(inner: &[Token]) -> Option<Color> {
    let (c, a) = channels(inner)?;
    let hue = match &c[0] {
        Token::Number(n) => *n,
        Token::Dimension(n, u) => match u.as_str() {
            "deg" => *n,
            "turn" => n * 360.0,
            "rad" => n.to_degrees(),
            "grad" => n * 0.9,
            _ => return None,
        },
        _ => return None,
    };
    let pct = |t: &Token| match t {
        Token::Percentage(p) => Some((p / 100.0).clamp(0.0, 1.0)),
        Token::Number(n) => Some((n / 100.0).clamp(0.0, 1.0)),
        _ => None,
    };
    let (s, l) = (pct(&c[1])?, pct(&c[2])?);
    let h = hue.rem_euclid(360.0) / 360.0;
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Some(Color([f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0), alpha(a)?]))
}

/// CSS's named colours.
fn named(name: &str) -> Option<u32> {
    const NAMES: &[(&str, u32)] = &[
        ("aliceblue", 0xf0f8ff),
        ("antiquewhite", 0xfaebd7),
        ("aqua", 0x00ffff),
        ("aquamarine", 0x7fffd4),
        ("azure", 0xf0ffff),
        ("beige", 0xf5f5dc),
        ("bisque", 0xffe4c4),
        ("black", 0x000000),
        ("blanchedalmond", 0xffebcd),
        ("blue", 0x0000ff),
        ("blueviolet", 0x8a2be2),
        ("brown", 0xa52a2a),
        ("burlywood", 0xdeb887),
        ("cadetblue", 0x5f9ea0),
        ("chartreuse", 0x7fff00),
        ("chocolate", 0xd2691e),
        ("coral", 0xff7f50),
        ("cornflowerblue", 0x6495ed),
        ("cornsilk", 0xfff8dc),
        ("crimson", 0xdc143c),
        ("cyan", 0x00ffff),
        ("darkblue", 0x00008b),
        ("darkcyan", 0x008b8b),
        ("darkgoldenrod", 0xb8860b),
        ("darkgray", 0xa9a9a9),
        ("darkgreen", 0x006400),
        ("darkgrey", 0xa9a9a9),
        ("darkkhaki", 0xbdb76b),
        ("darkmagenta", 0x8b008b),
        ("darkolivegreen", 0x556b2f),
        ("darkorange", 0xff8c00),
        ("darkorchid", 0x9932cc),
        ("darkred", 0x8b0000),
        ("darksalmon", 0xe9967a),
        ("darkseagreen", 0x8fbc8f),
        ("darkslateblue", 0x483d8b),
        ("darkslategray", 0x2f4f4f),
        ("darkslategrey", 0x2f4f4f),
        ("darkturquoise", 0x00ced1),
        ("darkviolet", 0x9400d3),
        ("deeppink", 0xff1493),
        ("deepskyblue", 0x00bfff),
        ("dimgray", 0x696969),
        ("dimgrey", 0x696969),
        ("dodgerblue", 0x1e90ff),
        ("firebrick", 0xb22222),
        ("floralwhite", 0xfffaf0),
        ("forestgreen", 0x228b22),
        ("fuchsia", 0xff00ff),
        ("gainsboro", 0xdcdcdc),
        ("ghostwhite", 0xf8f8ff),
        ("gold", 0xffd700),
        ("goldenrod", 0xdaa520),
        ("gray", 0x808080),
        ("green", 0x008000),
        ("greenyellow", 0xadff2f),
        ("grey", 0x808080),
        ("honeydew", 0xf0fff0),
        ("hotpink", 0xff69b4),
        ("indianred", 0xcd5c5c),
        ("indigo", 0x4b0082),
        ("ivory", 0xfffff0),
        ("khaki", 0xf0e68c),
        ("lavender", 0xe6e6fa),
        ("lavenderblush", 0xfff0f5),
        ("lawngreen", 0x7cfc00),
        ("lemonchiffon", 0xfffacd),
        ("lightblue", 0xadd8e6),
        ("lightcoral", 0xf08080),
        ("lightcyan", 0xe0ffff),
        ("lightgoldenrodyellow", 0xfafad2),
        ("lightgray", 0xd3d3d3),
        ("lightgreen", 0x90ee90),
        ("lightgrey", 0xd3d3d3),
        ("lightpink", 0xffb6c1),
        ("lightsalmon", 0xffa07a),
        ("lightseagreen", 0x20b2aa),
        ("lightskyblue", 0x87cefa),
        ("lightslategray", 0x778899),
        ("lightslategrey", 0x778899),
        ("lightsteelblue", 0xb0c4de),
        ("lightyellow", 0xffffe0),
        ("lime", 0x00ff00),
        ("limegreen", 0x32cd32),
        ("linen", 0xfaf0e6),
        ("magenta", 0xff00ff),
        ("maroon", 0x800000),
        ("mediumaquamarine", 0x66cdaa),
        ("mediumblue", 0x0000cd),
        ("mediumorchid", 0xba55d3),
        ("mediumpurple", 0x9370db),
        ("mediumseagreen", 0x3cb371),
        ("mediumslateblue", 0x7b68ee),
        ("mediumspringgreen", 0x00fa9a),
        ("mediumturquoise", 0x48d1cc),
        ("mediumvioletred", 0xc71585),
        ("midnightblue", 0x191970),
        ("mintcream", 0xf5fffa),
        ("mistyrose", 0xffe4e1),
        ("moccasin", 0xffe4b5),
        ("navajowhite", 0xffdead),
        ("navy", 0x000080),
        ("oldlace", 0xfdf5e6),
        ("olive", 0x808000),
        ("olivedrab", 0x6b8e23),
        ("orange", 0xffa500),
        ("orangered", 0xff4500),
        ("orchid", 0xda70d6),
        ("palegoldenrod", 0xeee8aa),
        ("palegreen", 0x98fb98),
        ("paleturquoise", 0xafeeee),
        ("palevioletred", 0xdb7093),
        ("papayawhip", 0xffefd5),
        ("peachpuff", 0xffdab9),
        ("peru", 0xcd853f),
        ("pink", 0xffc0cb),
        ("plum", 0xdda0dd),
        ("powderblue", 0xb0e0e6),
        ("purple", 0x800080),
        ("rebeccapurple", 0x663399),
        ("red", 0xff0000),
        ("rosybrown", 0xbc8f8f),
        ("royalblue", 0x4169e1),
        ("saddlebrown", 0x8b4513),
        ("salmon", 0xfa8072),
        ("sandybrown", 0xf4a460),
        ("seagreen", 0x2e8b57),
        ("seashell", 0xfff5ee),
        ("sienna", 0xa0522d),
        ("silver", 0xc0c0c0),
        ("skyblue", 0x87ceeb),
        ("slateblue", 0x6a5acd),
        ("slategray", 0x708090),
        ("slategrey", 0x708090),
        ("snow", 0xfffafa),
        ("springgreen", 0x00ff7f),
        ("steelblue", 0x4682b4),
        ("tan", 0xd2b48c),
        ("teal", 0x008080),
        ("thistle", 0xd8bfd8),
        ("tomato", 0xff6347),
        ("turquoise", 0x40e0d0),
        ("violet", 0xee82ee),
        ("wheat", 0xf5deb3),
        ("white", 0xffffff),
        ("whitesmoke", 0xf5f5f5),
        ("yellow", 0xffff00),
        ("yellowgreen", 0x9acd32),
    ];
    NAMES
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| NAMES[i].1)
}
