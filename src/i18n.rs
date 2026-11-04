//! Interface language. The .slint UI marks its strings with Slint's `@tr` and gets them from the
//! bundled translations; strings set from Rust go through [`tr`], which reads the same `.po` file
//! (entries with `msgctxt "Rust"`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

/// 0 English, 1 Turkish (the order of the language picker).
static LANGUAGE: AtomicI32 = AtomicI32::new(0);

const TURKISH_PO: &str = include_str!("../translations/tr/LC_MESSAGES/aio-screen.po");

/// Switch the UI and Rust-side strings. Call after the window exists.
pub fn set_language(language: i32) {
    let language = language.clamp(0, 1);
    LANGUAGE.store(language, Ordering::Relaxed);
    if let Err(e) = slint::select_bundled_translation(if language == 1 { "tr" } else { "" }) {
        eprintln!("Could not switch the UI language: {e}");
    }
}

/// The current translation of an English string, or the string itself.
pub fn tr(english: &'static str) -> &'static str {
    if LANGUAGE.load(Ordering::Relaxed) != 1 {
        return english;
    }
    static TURKISH: OnceLock<HashMap<String, String>> = OnceLock::new();
    TURKISH.get_or_init(|| parse_po(TURKISH_PO, "Rust")).get(english).map_or(english, |s| s.as_str())
}

/// msgid → msgstr for the entries with the given msgctxt. Empty translations are skipped.
fn parse_po(po: &str, context: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let (mut ctx, mut id, mut text) = (String::new(), String::new(), String::new());
    let mut field: Option<&str> = None;
    let mut flush = |ctx: &mut String, id: &mut String, text: &mut String| {
        if ctx == context && !id.is_empty() && !text.is_empty() {
            map.insert(std::mem::take(id), std::mem::take(text));
        }
        ctx.clear();
        id.clear();
        text.clear();
    };
    for line in po.lines().map(str::trim) {
        let (key, rest) = match line.split_once(' ') {
            Some((k, r)) if matches!(k, "msgctxt" | "msgid" | "msgstr") => (Some(k), r),
            _ if line.starts_with('"') => (None, line),
            _ => {
                if line.is_empty() { flush(&mut ctx, &mut id, &mut text); }
                field = None;
                continue;
            }
        };
        if key == Some("msgctxt") || (key == Some("msgid") && field == Some("msgstr")) {
            flush(&mut ctx, &mut id, &mut text);
        }
        if key.is_some() { field = key; }
        let target = match field {
            Some("msgctxt") => &mut ctx,
            Some("msgid") => &mut id,
            Some("msgstr") => &mut text,
            _ => continue,
        };
        target.push_str(&unquote(rest));
    }
    flush(&mut ctx, &mut id, &mut text);
    map
}

fn unquote(s: &str) -> String {
    let inner = s.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or("");
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue; }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_context_and_escapes() {
        let po = "msgid \"\"\nmsgstr \"Language: tr\\n\"\n\n\
                  msgctxt \"Rust\"\nmsgid \"Quit\"\nmsgstr \"Çıkış\"\n\n\
                  msgctxt \"AppWindow\"\nmsgid \"Quit\"\nmsgstr \"Başka\"\n\n\
                  msgctxt \"Rust\"\nmsgid \"\"\n\"Say \\\"hi\\\"\"\nmsgstr \"\\\"Selam\\\"\\nde\"\n";
        let map = parse_po(po, "Rust");
        assert_eq!(map.len(), 2);
        assert_eq!(map["Quit"], "Çıkış");
        assert_eq!(map["Say \"hi\""], "\"Selam\"\nde");
    }

    #[test]
    fn every_rust_string_is_translated() {
        let map = parse_po(TURKISH_PO, "Rust");
        assert!(map.len() > 40, "only {} Rust entries", map.len());
        assert!(map.values().all(|v| !v.is_empty()));
    }
}
