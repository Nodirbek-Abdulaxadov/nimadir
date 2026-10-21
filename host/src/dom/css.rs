//! CSS, read by the host (ADR-4): a tokenizer, then stylesheets — rules,
//! selectors, `@media` — with each declaration kept as tokens until the page
//! is styled, when `var()` can be put in and the value read for its property.
//!
//! A subset, and forgiving the way browsers are: a rule that cannot be read is
//! skipped with a warning, and the rest of the sheet still applies. Nothing
//! here recurses on the guest's say: blocks are found by counting brackets,
//! and `@media` nests at most `MAX_MEDIA_NESTING` deep.

/// How deep `@media` blocks may nest inside each other.
const MAX_MEDIA_NESTING: usize = 4;

/// How many warnings one sheet logs before it only counts them.
pub const MAX_WARNINGS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Ident(String),
    /// A name and its `(`; the arguments follow, then `RParen`.
    Function(String),
    AtKeyword(String),
    Hash(String),
    Str(String),
    Number(f32),
    Percentage(f32),
    /// A number and its unit, lowercase.
    Dimension(f32, String),
    Delim(char),
    Colon,
    Semicolon,
    Comma,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Space,
}

impl Token {
    pub fn ident(&self) -> Option<&str> {
        match self {
            Token::Ident(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_ident(&self, name: &str) -> bool {
        self.ident().is_some_and(|s| s.eq_ignore_ascii_case(name))
    }

    /// The bracket this one closes, if it opens one.
    fn closer(&self) -> Option<Token> {
        match self {
            Token::Function(_) | Token::LParen => Some(Token::RParen),
            Token::LBrace => Some(Token::RBrace),
            Token::LBracket => Some(Token::RBracket),
            _ => None,
        }
    }
}

/// Write tokens back as CSS, for warnings and for printing values.
pub fn to_css(tokens: &[Token]) -> String {
    let mut out = String::new();
    for t in tokens {
        match t {
            Token::Ident(s) => out.push_str(s),
            Token::Function(s) => {
                out.push_str(s);
                out.push('(');
            }
            Token::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            }
            Token::Hash(s) => {
                out.push('#');
                out.push_str(s);
            }
            Token::Str(s) => out.push_str(&format!("{s:?}")),
            Token::Number(n) => out.push_str(&number(*n)),
            Token::Percentage(n) => {
                out.push_str(&number(*n));
                out.push('%');
            }
            Token::Dimension(n, u) => {
                out.push_str(&number(*n));
                out.push_str(u);
            }
            Token::Delim(c) => out.push(*c),
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            Token::Comma => out.push(','),
            Token::LBrace => out.push('{'),
            Token::RBrace => out.push('}'),
            Token::LParen => out.push('('),
            Token::RParen => out.push(')'),
            Token::LBracket => out.push('['),
            Token::RBracket => out.push(']'),
            Token::Space => out.push(' '),
        }
    }
    out
}

/// A number as CSS writes it: no trailing zeros, no exponent.
pub fn number(n: f32) -> String {
    if n.fract() == 0.0 && n.abs() < 1e9 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_name(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

/// Split CSS into tokens. Comments go; runs of whitespace become one `Space`.
pub fn tokenize(css: &str) -> Vec<Token> {
    let chars: Vec<char> = css.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while i < chars.len() {
        let c = chars[i];
        // Comments.
        if c == '/' && at(i + 1) == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && at(i + 1) == Some('/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c.is_whitespace() {
            while at(i).is_some_and(char::is_whitespace) {
                i += 1;
            }
            if out.last() != Some(&Token::Space) {
                out.push(Token::Space);
            }
            continue;
        }
        if c == '"' || c == '\'' {
            let (s, next) = string(&chars, i + 1, c);
            out.push(Token::Str(s));
            i = next;
            continue;
        }
        let starts_number = c.is_ascii_digit()
            || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit()))
            || ((c == '+' || c == '-')
                && (at(i + 1).is_some_and(|d| d.is_ascii_digit())
                    || (at(i + 1) == Some('.') && at(i + 2).is_some_and(|d| d.is_ascii_digit()))));
        if starts_number {
            let (n, next) = numeric(&chars, i);
            i = next;
            if at(i) == Some('%') {
                out.push(Token::Percentage(n));
                i += 1;
            } else if at(i).is_some_and(is_name_start)
                || (at(i) == Some('-') && at(i + 1).is_some_and(is_name_start))
            {
                let (unit, next) = name(&chars, i);
                out.push(Token::Dimension(n, unit.to_ascii_lowercase()));
                i = next;
            } else {
                out.push(Token::Number(n));
            }
            continue;
        }
        let starts_name = is_name_start(c)
            || c == '\\'
            || (c == '-' && at(i + 1).is_some_and(|d| is_name_start(d) || d == '-'));
        if starts_name {
            let (word, next) = name(&chars, i);
            i = next;
            if at(i) == Some('(') {
                out.push(Token::Function(word.to_ascii_lowercase()));
                i += 1;
            } else {
                out.push(Token::Ident(word));
            }
            continue;
        }
        if c == '@' && at(i + 1).is_some_and(|d| is_name_start(d) || d == '-') {
            let (word, next) = name(&chars, i + 1);
            out.push(Token::AtKeyword(word.to_ascii_lowercase()));
            i = next;
            continue;
        }
        if c == '#' && at(i + 1).is_some_and(|d| is_name(d) || d == '\\') {
            let (word, next) = name(&chars, i + 1);
            out.push(Token::Hash(word));
            i = next;
            continue;
        }
        out.push(match c {
            ':' => Token::Colon,
            ';' => Token::Semicolon,
            ',' => Token::Comma,
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            other => Token::Delim(other),
        });
        i += 1;
    }
    out
}

/// A quoted string from just after its opening quote; returns where it ended.
fn string(chars: &[char], mut i: usize, quote: char) -> (String, usize) {
    let mut s = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == quote {
            return (s, i + 1);
        }
        if c == '\\' && i + 1 < chars.len() {
            let (ch, next) = escape(chars, i + 1);
            s.push(ch);
            i = next;
            continue;
        }
        if c == '\n' {
            // An unclosed string ends at the line, as in a browser.
            return (s, i);
        }
        s.push(c);
        i += 1;
    }
    (s, i)
}

/// What follows a backslash: a hex code point, or the character itself.
fn escape(chars: &[char], mut i: usize) -> (char, usize) {
    let start = i;
    while i < chars.len() && i - start < 6 && chars[i].is_ascii_hexdigit() {
        i += 1;
    }
    if i > start {
        let hex: String = chars[start..i].iter().collect();
        if chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        let ch = u32::from_str_radix(&hex, 16)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\u{fffd}');
        return (ch, i);
    }
    (chars[i], i + 1)
}

fn name(chars: &[char], mut i: usize) -> (String, usize) {
    let mut s = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let (ch, next) = escape(chars, i + 1);
            s.push(ch);
            i = next;
        } else if is_name(c) {
            s.push(c);
            i += 1;
        } else {
            break;
        }
    }
    (s, i)
}

fn numeric(chars: &[char], mut i: usize) -> (f32, usize) {
    let start = i;
    if matches!(chars.get(i), Some('+' | '-')) {
        i += 1;
    }
    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
        i += 1;
    }
    if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
        i += 1;
        while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+' | '-')) {
            j += 1;
        }
        if chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
            i = j;
            while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
        }
    }
    let text: String = chars[start..i].iter().collect();
    let n: f32 = text.parse().unwrap_or(0.0);
    (if n.is_finite() { n } else { 0.0 }, i)
}

/// From an opening bracket at `start`, the index of the token that closes it
/// — or the end, for one never closed.
fn matching(tokens: &[Token], start: usize) -> usize {
    let mut stack: Vec<Token> = Vec::new();
    for (i, t) in tokens.iter().enumerate().skip(start) {
        if let Some(close) = t.closer() {
            stack.push(close);
        } else if stack.last() == Some(t) {
            stack.pop();
            if stack.is_empty() {
                return i;
            }
        }
    }
    tokens.len()
}

/// Split at `sep` where no bracket is open.
pub fn split_top(tokens: &[Token], sep: &Token) -> Vec<Vec<Token>> {
    let mut parts = vec![Vec::new()];
    let mut depth = 0usize;
    for t in tokens {
        if t.closer().is_some() {
            depth += 1;
        } else if matches!(t, Token::RParen | Token::RBrace | Token::RBracket) {
            depth = depth.saturating_sub(1);
        }
        if depth == 0 && t == sep {
            parts.push(Vec::new());
        } else {
            parts.last_mut().expect("never empty").push(t.clone());
        }
    }
    parts
}

pub fn trim(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| *t != Token::Space)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| *t != Token::Space)
        .map_or(start, |e| e + 1);
    &tokens[start..end]
}

/// One `property: value` — the value as tokens, read when the page is styled.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// Lowercase, except a custom property (`--Name`), which is as written.
    pub name: String,
    pub value: Vec<Token>,
    pub important: bool,
}

/// The declarations of a block, or of a `style` attribute.
pub fn declarations(tokens: &[Token]) -> Vec<Declaration> {
    let mut out = Vec::new();
    for part in split_top(tokens, &Token::Semicolon) {
        let part = trim(&part);
        let Some(Token::Ident(name)) = part.first() else {
            continue;
        };
        let rest = trim(&part[1..]);
        if rest.first() != Some(&Token::Colon) {
            continue;
        }
        let mut value = trim(&rest[1..]).to_vec();
        let mut important = false;
        if let [.., Token::Delim('!'), last] = value.as_slice() {
            if last.is_ident("important") {
                important = true;
                value.truncate(value.len() - 2);
                value = trim(&value).to_vec();
            }
        } else if let [.., Token::Delim('!'), Token::Space, last] = value.as_slice() {
            if last.is_ident("important") {
                important = true;
                value.truncate(value.len() - 3);
                value = trim(&value).to_vec();
            }
        }
        // A custom property may be empty; nothing else may.
        let custom = name.starts_with("--");
        if value.is_empty() && !custom {
            continue;
        }
        out.push(Declaration {
            name: if custom {
                name.clone()
            } else {
                name.to_ascii_lowercase()
            },
            value,
            important,
        });
    }
    out
}

/// Inline style: `"padding: 4px; color: red"`.
pub fn inline(css: &str) -> Vec<Declaration> {
    declarations(&tokenize(css))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Combinator {
    /// `a b`
    Descendant,
    /// `a > b`
    Child,
    /// `a + b`
    Next,
    /// `a ~ b`
    Later,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrOp {
    Exists,
    Equals(String),
    /// `~=`: one of the space-separated words.
    Word(String),
    /// `|=`: the value, or it followed by `-`.
    Dash(String),
    Prefix(String),
    Suffix(String),
    Contains(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pseudo {
    Hover,
    Active,
    Focus,
    FocusWithin,
    Disabled,
    Enabled,
    Checked,
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    /// `:nth-child(an+b)`
    NthChild(i32, i32),
    NthLastChild(i32, i32),
    /// `:not(…)`, of simple compounds.
    Not(Vec<Compound>),
    Placeholder,
}

/// What one element must be: `button.primary:hover`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
}

impl Compound {
    fn specificity(&self) -> (u32, u32, u32) {
        let mut s = (
            self.id.is_some() as u32,
            (self.classes.len() + self.attrs.len()) as u32,
            self.tag.is_some() as u32,
        );
        for p in &self.pseudos {
            match p {
                Pseudo::Not(inner) => {
                    // As the most specific of what it lists.
                    let most = inner
                        .iter()
                        .map(Compound::specificity)
                        .max()
                        .unwrap_or((0, 0, 0));
                    s = (s.0 + most.0, s.1 + most.1, s.2 + most.2);
                }
                _ => s.1 += 1,
            }
        }
        s
    }
}

/// A complex selector, kept right to left: `compounds[0]` is the element
/// the rule styles, and `combinators[i]` says how `compounds[i + 1]` stands
/// to `compounds[i]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    /// Ids, then classes, attributes and pseudo-classes, then tags, packed
    /// so that a plain comparison orders them.
    pub specificity: u32,
}

/// A selector list, `h1, .title > span`; `None` if any of it cannot be
/// read, which drops the whole rule, as in a browser.
pub fn selectors(tokens: &[Token]) -> Option<Vec<Selector>> {
    split_top(tokens, &Token::Comma)
        .iter()
        .map(|part| selector(trim(part)))
        .collect()
}

pub fn selector(tokens: &[Token]) -> Option<Selector> {
    if tokens.is_empty() {
        return None;
    }
    let mut compounds: Vec<Compound> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut current = Compound::default();
    let mut has_current = false;
    let mut pending: Option<Combinator> = None;
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        let combinator = match t {
            Token::Space => Some(Combinator::Descendant),
            Token::Delim('>') => Some(Combinator::Child),
            Token::Delim('+') => Some(Combinator::Next),
            Token::Delim('~') => Some(Combinator::Later),
            _ => None,
        };
        if let Some(c) = combinator {
            if has_current {
                compounds.push(std::mem::take(&mut current));
                has_current = false;
                pending = Some(c);
            } else if c != Combinator::Descendant {
                // `a > b`: the spaces around it said descendant first.
                match pending {
                    Some(Combinator::Descendant) => pending = Some(c),
                    _ => return None,
                }
            }
            i += 1;
            continue;
        }
        if !has_current {
            if let Some(c) = pending.take() {
                combinators.push(c);
            } else if !compounds.is_empty() {
                return None;
            }
        }
        has_current = true;
        let used = simple(&mut current, &tokens[i..])?;
        i += used;
    }
    if !has_current {
        return None;
    }
    compounds.push(current);
    if compounds.len() > super::select::MAX_COMPOUNDS {
        return None;
    }
    compounds.reverse();
    combinators.reverse();
    let (a, b, c) = compounds
        .iter()
        .map(Compound::specificity)
        .fold((0, 0, 0), |s, x| (s.0 + x.0, s.1 + x.1, s.2 + x.2));
    Some(Selector {
        compounds,
        combinators,
        specificity: (a.min(1023) << 20) | (b.min(1023) << 10) | c.min(1023),
    })
}

/// One simple selector into `c`; how many tokens it took, or `None` for one
/// that cannot be read.
fn simple(c: &mut Compound, tokens: &[Token]) -> Option<usize> {
    match &tokens[0] {
        Token::Ident(tag) if c.tag.is_none() => {
            c.tag = Some(tag.to_ascii_lowercase());
            Some(1)
        }
        Token::Delim('*') => Some(1),
        Token::Hash(id) if c.id.is_none() => {
            c.id = Some(id.clone());
            Some(1)
        }
        Token::Delim('.') => match tokens.get(1) {
            Some(Token::Ident(class)) => {
                c.classes.push(class.clone());
                Some(2)
            }
            _ => None,
        },
        Token::LBracket => {
            let end = matching(tokens, 0);
            if end >= tokens.len() {
                return None;
            }
            c.attrs.push(attribute(trim(&tokens[1..end]))?);
            Some(end + 1)
        }
        Token::Colon => match tokens.get(1) {
            // `::before` and the like: a box this host does not make.
            Some(Token::Colon) => None,
            Some(Token::Ident(name)) => {
                c.pseudos.push(pseudo(name, None)?);
                Some(2)
            }
            Some(Token::Function(name)) => {
                let end = matching(tokens, 1);
                if end >= tokens.len() {
                    return None;
                }
                c.pseudos.push(pseudo(name, Some(trim(&tokens[2..end])))?);
                Some(end + 1)
            }
            _ => None,
        },
        _ => None,
    }
}

fn attribute(tokens: &[Token]) -> Option<AttrSel> {
    let Some(Token::Ident(name)) = tokens.first() else {
        return None;
    };
    let name = name.to_ascii_lowercase();
    let rest = trim(&tokens[1..]);
    if rest.is_empty() {
        return Some(AttrSel {
            name,
            op: AttrOp::Exists,
        });
    }
    let (op, value) = match rest {
        [Token::Delim('='), value @ ..] => ("=", value),
        [Token::Delim(c), Token::Delim('='), value @ ..] => match c {
            '~' => ("~=", value),
            '|' => ("|=", value),
            '^' => ("^=", value),
            '$' => ("$=", value),
            '*' => ("*=", value),
            _ => return None,
        },
        _ => return None,
    };
    let value = match trim(value) {
        [Token::Ident(v)] | [Token::Str(v)] => v.clone(),
        // `[i]` and `[s]` flags are not read; the value alone is.
        [Token::Ident(v) | Token::Str(v), Token::Space, Token::Ident(_)] => v.clone(),
        [Token::Number(n)] => number(*n),
        _ => return None,
    };
    let op = match op {
        "=" => AttrOp::Equals(value),
        "~=" => AttrOp::Word(value),
        "|=" => AttrOp::Dash(value),
        "^=" => AttrOp::Prefix(value),
        "$=" => AttrOp::Suffix(value),
        _ => AttrOp::Contains(value),
    };
    Some(AttrSel { name, op })
}

fn pseudo(name: &str, args: Option<&[Token]>) -> Option<Pseudo> {
    let name = name.to_ascii_lowercase();
    Some(match (name.as_str(), args) {
        ("hover", None) => Pseudo::Hover,
        ("active", None) => Pseudo::Active,
        ("focus", None) | ("focus-visible", None) => Pseudo::Focus,
        ("focus-within", None) => Pseudo::FocusWithin,
        ("disabled", None) => Pseudo::Disabled,
        ("enabled", None) => Pseudo::Enabled,
        ("checked", None) => Pseudo::Checked,
        ("root", None) => Pseudo::Root,
        ("empty", None) => Pseudo::Empty,
        ("first-child", None) => Pseudo::FirstChild,
        ("last-child", None) => Pseudo::LastChild,
        ("only-child", None) => Pseudo::OnlyChild,
        ("placeholder-shown", None) => Pseudo::Placeholder,
        ("nth-child", Some(a)) => {
            let (a, b) = nth(a)?;
            Pseudo::NthChild(a, b)
        }
        ("nth-last-child", Some(a)) => {
            let (a, b) = nth(a)?;
            Pseudo::NthLastChild(a, b)
        }
        ("not", Some(a)) => {
            let mut inner = Vec::new();
            for part in split_top(a, &Token::Comma) {
                let part = trim(&part);
                if part.is_empty() {
                    return None;
                }
                let mut c = Compound::default();
                let mut i = 0;
                while i < part.len() {
                    i += simple(&mut c, &part[i..])?;
                }
                inner.push(c);
            }
            Pseudo::Not(inner)
        }
        _ => return None,
    })
}

/// `an+b` for `:nth-child`: `odd`, `even`, `3`, `2n`, `2n+1`, `-n+3`.
///
/// Read back into text first, because the tokenizer splits it oddly — `2n-1`
/// is one dimension with the unit `n-1`, `2n+1` a dimension and a number —
/// and a number after the `n` part can only have had a sign in front of it.
fn nth(tokens: &[Token]) -> Option<(i32, i32)> {
    let mut text = String::new();
    for t in tokens {
        match t {
            Token::Space => {}
            Token::Number(n) => {
                let signed = text.ends_with('+') || text.ends_with('-');
                if !text.is_empty() && !signed && *n >= 0.0 {
                    text.push('+');
                }
                text.push_str(&number(*n));
            }
            Token::Dimension(n, u) => {
                text.push_str(&number(*n));
                text.push_str(u);
            }
            Token::Ident(s) => text.push_str(s),
            Token::Delim(c) => text.push(*c),
            _ => return None,
        }
    }
    let text = text.to_ascii_lowercase();
    match text.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    let Some(n_at) = text.find('n') else {
        return text.parse().ok().map(|b| (0, b));
    };
    let a = match &text[..n_at] {
        "" | "+" => 1,
        "-" => -1,
        s => s.parse().ok()?,
    };
    let b = match &text[n_at + 1..] {
        "" => 0,
        s => s.strip_prefix('+').unwrap_or(s).parse().ok()?,
    };
    Some((a, b))
}

/// What the page is shown on, for `@media`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Env {
    pub width: f32,
    pub height: f32,
    pub dark: bool,
    /// A mouse that can hover; false on a touch screen.
    pub hover: bool,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            width: 800.0,
            height: 600.0,
            dark: false,
            hover: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Feature {
    Dark(bool),
    MinWidth(f32),
    MaxWidth(f32),
    MinHeight(f32),
    MaxHeight(f32),
    Portrait(bool),
    Hover(bool),
    /// A feature this host does not know: never true.
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
struct MediaQuery {
    not: bool,
    /// `print` and other types this host is not.
    other_type: bool,
    features: Vec<Feature>,
}

/// A media query list: true when any query in it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Media(Vec<MediaQuery>);

impl Media {
    pub fn matches(&self, env: &Env) -> bool {
        self.0.iter().any(|q| {
            let yes = !q.other_type
                && q.features.iter().all(|f| match f {
                    Feature::Dark(d) => env.dark == *d,
                    Feature::MinWidth(w) => env.width >= *w,
                    Feature::MaxWidth(w) => env.width <= *w,
                    Feature::MinHeight(h) => env.height >= *h,
                    Feature::MaxHeight(h) => env.height <= *h,
                    Feature::Portrait(p) => (env.height >= env.width) == *p,
                    Feature::Hover(h) => env.hover == *h,
                    Feature::Unknown => false,
                });
            yes != q.not
        })
    }
}

fn media(tokens: &[Token]) -> Media {
    let mut queries = Vec::new();
    for part in split_top(tokens, &Token::Comma) {
        let part = trim(&part);
        let mut q = MediaQuery {
            not: false,
            other_type: false,
            features: Vec::new(),
        };
        let mut i = 0;
        while i < part.len() {
            match &part[i] {
                Token::Space => {}
                Token::Ident(w) if w.eq_ignore_ascii_case("not") => q.not = true,
                Token::Ident(w)
                    if w.eq_ignore_ascii_case("only") || w.eq_ignore_ascii_case("and") => {}
                Token::Ident(w)
                    if w.eq_ignore_ascii_case("all") || w.eq_ignore_ascii_case("screen") => {}
                Token::Ident(_) => q.other_type = true,
                Token::LParen => {
                    let end = matching(part, i);
                    q.features
                        .push(feature(trim(&part[i + 1..end.min(part.len())])));
                    i = end;
                }
                _ => q.features.push(Feature::Unknown),
            }
            i += 1;
        }
        queries.push(q);
    }
    Media(queries)
}

fn feature(tokens: &[Token]) -> Feature {
    let parts = split_top(tokens, &Token::Colon);
    let name = trim(&parts[0])
        .first()
        .and_then(Token::ident)
        .unwrap_or("")
        .to_ascii_lowercase();
    let value = parts.get(1).map(|v| trim(v).to_vec()).unwrap_or_default();
    let px = || match value.as_slice() {
        [Token::Dimension(n, u)] if u == "px" => Some(*n),
        [Token::Dimension(n, u)] if u == "em" || u == "rem" => Some(*n * 16.0),
        [Token::Number(n)] if *n == 0.0 => Some(0.0),
        _ => None,
    };
    let word = value
        .first()
        .and_then(Token::ident)
        .map(str::to_ascii_lowercase);
    match (name.as_str(), word.as_deref()) {
        ("prefers-color-scheme", Some("dark")) => Feature::Dark(true),
        ("prefers-color-scheme", Some("light")) => Feature::Dark(false),
        ("orientation", Some("portrait")) => Feature::Portrait(true),
        ("orientation", Some("landscape")) => Feature::Portrait(false),
        ("hover", Some("hover")) | ("any-hover", Some("hover")) => Feature::Hover(true),
        ("hover", Some("none")) | ("any-hover", Some("none")) => Feature::Hover(false),
        ("pointer", Some("fine")) => Feature::Hover(true),
        ("pointer", Some("coarse")) => Feature::Hover(false),
        ("min-width", _) => px().map_or(Feature::Unknown, Feature::MinWidth),
        ("max-width", _) => px().map_or(Feature::Unknown, Feature::MaxWidth),
        ("min-height", _) => px().map_or(Feature::Unknown, Feature::MinHeight),
        ("max-height", _) => px().map_or(Feature::Unknown, Feature::MaxHeight),
        _ => Feature::Unknown,
    }
}

/// One rule: its selectors, its declarations, the `@media` it sits in.
#[derive(Clone, Debug)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// Every enclosing `@media`, all of which must match.
    pub media: Vec<Media>,
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub rules: Vec<Rule>,
    /// What was skipped and why, for the app's log.
    pub warnings: Vec<String>,
    /// How many warnings there were past `MAX_WARNINGS`.
    pub more_warnings: usize,
}

impl Sheet {
    fn warn(&mut self, what: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(what);
        } else {
            self.more_warnings += 1;
        }
    }
}

pub fn parse(css: &str) -> Sheet {
    let tokens = tokenize(css);
    let mut sheet = Sheet::default();
    rules(&tokens, &[], 0, &mut sheet);
    sheet
}

/// The rules in `tokens`, under the `@media` list `within`.
fn rules(tokens: &[Token], within: &[Media], nesting: usize, sheet: &mut Sheet) {
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Space | Token::Semicolon => i += 1,
            Token::AtKeyword(name) => {
                // The prelude runs to the block or the semicolon.
                let start = i + 1;
                let mut j = start;
                while j < tokens.len() && !matches!(tokens[j], Token::LBrace | Token::Semicolon) {
                    j += 1;
                }
                if tokens.get(j) != Some(&Token::LBrace) {
                    if name != "charset" {
                        sheet.warn(format!("skipped @{name}: not read by this host"));
                    }
                    i = j + 1;
                    continue;
                }
                let end = matching(tokens, j);
                if name == "media" && nesting < MAX_MEDIA_NESTING {
                    let mut inner = within.to_vec();
                    inner.push(media(trim(&tokens[start..j])));
                    rules(
                        &tokens[j + 1..end.min(tokens.len())],
                        &inner,
                        nesting + 1,
                        sheet,
                    );
                } else {
                    sheet.warn(format!("skipped @{name}: not read by this host"));
                }
                i = end + 1;
            }
            _ => {
                let mut j = i;
                while j < tokens.len() && tokens[j] != Token::LBrace {
                    j += 1;
                }
                if j >= tokens.len() {
                    sheet.warn(format!(
                        "skipped `{}`: no block",
                        to_css(trim(&tokens[i..]))
                    ));
                    return;
                }
                let end = matching(tokens, j);
                let prelude = trim(&tokens[i..j]);
                let block = &tokens[j + 1..end.min(tokens.len())];
                match selectors(prelude) {
                    Some(selectors) => sheet.rules.push(Rule {
                        selectors,
                        declarations: declarations(block),
                        media: within.to_vec(),
                    }),
                    None => sheet.warn(format!(
                        "skipped `{}`: a selector this host does not read",
                        to_css(prelude)
                    )),
                }
                i = end + 1;
            }
        }
    }
}
