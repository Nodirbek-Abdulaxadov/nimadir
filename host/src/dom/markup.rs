//! Markup for `set-markup` (ADR-4): an HTML-like snippet read into a tree of
//! elements and text, before any of it touches the document — so markup
//! that cannot be read changes nothing, and the error says where.
//!
//! Elements with attributes (quoted, unquoted or bare), text with entities,
//! void elements (`br`, `img`, `input`, …) that need no closing tag,
//! self-closing `<x/>`, comments and `<!doctype>`. `script` and `style`
//! elements are dropped with what is in them: no scripts run here, and
//! stylesheets go through `add-stylesheet`. It is read with an explicit
//! stack, never recursion, and refused past `MAX_DEPTH` levels.

use super::MAX_DEPTH;

/// One node of parsed markup.
#[derive(Clone, Debug, PartialEq)]
pub enum Markup {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<Markup>,
    },
    Text(String),
}

/// Elements that never have content, so never a closing tag.
fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "hr"
            | "img"
            | "input"
            | "meta"
            | "link"
            | "area"
            | "base"
            | "col"
            | "embed"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Where in the markup `at` is, as a person counts: `line 3, column 7`.
fn place(src: &str, at: usize) -> String {
    let before = &src[..at.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("line {line}, column {column}")
}

/// An element still open while reading: its tag, attributes, children so far,
/// and where it opened.
type Open = (String, Vec<(String, String)>, Vec<Markup>, usize);

/// The top-level nodes `src` describes.
pub fn parse(src: &str) -> Result<Vec<Markup>, String> {
    // Open elements, innermost last: tag, attributes, children so far.
    let mut stack: Vec<Open> = Vec::new();
    let mut top: Vec<Markup> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let push = |stack: &mut Vec<Open>, top: &mut Vec<Markup>, node: Markup| match stack.last_mut() {
        Some((_, _, children, _)) => children.push(node),
        None => top.push(node),
    };
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = src[i..].find('<').map_or(src.len(), |n| i + n);
            let text = decode(&src[i..end]);
            if !text.is_empty() {
                push(&mut stack, &mut top, Markup::Text(text));
            }
            i = end;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            let end = rest
                .find("-->")
                .ok_or_else(|| format!("a comment is never closed, at {}", place(src, i)))?;
            i += end + 3;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            let end = rest
                .find('>')
                .ok_or_else(|| format!("`<!` is never closed, at {}", place(src, i)))?;
            i += end + 1;
            continue;
        }
        if let Some(close) = rest.strip_prefix("</") {
            let end = close
                .find('>')
                .ok_or_else(|| format!("a closing tag is never finished, at {}", place(src, i)))?;
            let tag = close[..end].trim().to_ascii_lowercase();
            match stack.pop() {
                Some((open, attrs, children, _)) if open == tag => {
                    push(
                        &mut stack,
                        &mut top,
                        Markup::Element {
                            tag,
                            attrs,
                            children,
                        },
                    );
                }
                Some((open, _, _, _)) => {
                    return Err(format!(
                        "`</{tag}>` closes `<{open}>`, at {}",
                        place(src, i)
                    ));
                }
                None => return Err(format!("`</{tag}>` closes nothing, at {}", place(src, i))),
            }
            i += 2 + end + 1;
            continue;
        }
        let (tag, attrs, self_closing, len) = open_tag(src, i)?;
        i += len;
        if tag == "script" || tag == "style" {
            // Dropped, with everything up to its closing tag.
            let closing = format!("</{tag}");
            let end = src[i..]
                .to_ascii_lowercase()
                .find(&closing)
                .ok_or_else(|| format!("`<{tag}>` is never closed, at {}", place(src, i)))?;
            let after = src[i + end..]
                .find('>')
                .map_or(src.len(), |n| i + end + n + 1);
            i = after;
            continue;
        }
        if self_closing || is_void(&tag) {
            push(
                &mut stack,
                &mut top,
                Markup::Element {
                    tag,
                    attrs,
                    children: Vec::new(),
                },
            );
            continue;
        }
        if stack.len() >= MAX_DEPTH {
            return Err(format!(
                "nested more than {MAX_DEPTH} levels deep, at {}",
                place(src, i)
            ));
        }
        stack.push((tag, attrs, Vec::new(), i));
    }
    if let Some((open, _, _, at)) = stack.last() {
        return Err(format!(
            "`<{open}>` is never closed, opened at {}",
            place(src, *at)
        ));
    }
    Ok(top)
}

/// The opening tag at `start`: its name, attributes, whether it closes
/// itself, and how long it is.
#[allow(clippy::type_complexity)]
fn open_tag(
    src: &str,
    start: usize,
) -> Result<(String, Vec<(String, String)>, bool, usize), String> {
    let chars: Vec<(usize, char)> = src[start..].char_indices().collect();
    let at = |k: usize| chars.get(k).map(|(_, c)| *c);
    let mut k = 1;
    let name_start = k;
    while at(k).is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':') {
        k += 1;
    }
    if k == name_start {
        return Err(format!("`<` begins no tag, at {}", place(src, start)));
    }
    let tag: String = chars[name_start..k]
        .iter()
        .map(|(_, c)| *c)
        .collect::<String>()
        .to_ascii_lowercase();
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        while at(k).is_some_and(char::is_whitespace) {
            k += 1;
        }
        match at(k) {
            None => {
                return Err(format!(
                    "`<{tag}` is never finished, at {}",
                    place(src, start)
                ))
            }
            Some('>') => return Ok((tag, attrs, false, chars[k].0 + 1)),
            Some('/') if at(k + 1) == Some('>') => {
                return Ok((tag, attrs, true, chars[k + 1].0 + 1))
            }
            _ => {}
        }
        let name_start = k;
        while at(k)
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '=' | '>' | '/' | '"' | '\''))
        {
            k += 1;
        }
        if k == name_start {
            return Err(format!(
                "an attribute of `<{tag}>` has no name, at {}",
                place(src, start + chars[k].0)
            ));
        }
        let name: String = chars[name_start..k]
            .iter()
            .map(|(_, c)| *c)
            .collect::<String>()
            .to_ascii_lowercase();
        while at(k).is_some_and(char::is_whitespace) {
            k += 1;
        }
        let mut value = String::new();
        if at(k) == Some('=') {
            k += 1;
            while at(k).is_some_and(char::is_whitespace) {
                k += 1;
            }
            match at(k) {
                Some(q @ ('"' | '\'')) => {
                    k += 1;
                    let v_start = k;
                    while at(k).is_some_and(|c| c != q) {
                        k += 1;
                    }
                    if at(k).is_none() {
                        return Err(format!(
                            "a value in `<{tag}>` is never closed, at {}",
                            place(src, start)
                        ));
                    }
                    value = decode(
                        &chars[v_start..k]
                            .iter()
                            .map(|(_, c)| *c)
                            .collect::<String>(),
                    );
                    k += 1;
                }
                _ => {
                    let v_start = k;
                    while at(k).is_some_and(|c| !c.is_whitespace() && c != '>') {
                        k += 1;
                    }
                    value = decode(
                        &chars[v_start..k]
                            .iter()
                            .map(|(_, c)| *c)
                            .collect::<String>(),
                    );
                }
            }
        }
        if !attrs.iter().any(|(n, _)| *n == name) {
            attrs.push((name, value));
        }
    }
}

/// Text with its entities put back: `&amp;`, `&lt;`, `&#233;`, `&#x1F600;`, …
/// An entity this does not know stays as written.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(';').map(|n| n + 1).filter(|n| *n <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "copy" => Some('©'),
                "times" => Some('×'),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
