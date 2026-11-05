//! Interface language selection and translation of user-facing strings.
//!
//! Source strings in the code base stay in Simplified Chinese; they are the
//! lookup keys. `translate` returns the text for the active language:
//!
//! 1. exact match of the whole string;
//! 2. `format!` templates: a key such as `保存失败：{err}` matches the already
//!    formatted text, the captured values are translated recursively and put
//!    into the translated template;
//! 3. multi-part text joined with `\n`, ` · ` or `；` is translated per part.
//!
//! Anything without a translation is shown unchanged, so a missing entry never
//! breaks the UI. Chinese is the default, so behaviour without an explicit
//! language choice (CLI, tests) is unchanged.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

mod dict;

/// Interface language of the Aura GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    #[default]
    Zh,
    En,
    Ru,
    Nl,
}

impl Language {
    pub const ALL: [Language; 4] = [Language::Zh, Language::En, Language::Ru, Language::Nl];

    /// Stable value stored in `ui.toml`.
    pub fn code(self) -> &'static str {
        match self {
            Language::Zh => "zh-CN",
            Language::En => "en",
            Language::Ru => "ru",
            Language::Nl => "nl",
        }
    }

    /// Parse a stored code or a BCP-47 / Windows locale name (`ru-RU`, `nl-BE`…).
    pub fn from_code(code: &str) -> Option<Self> {
        let lower = code.trim().to_ascii_lowercase();
        let primary = lower.split(['-', '_']).next().unwrap_or("");
        match primary {
            "zh" => Some(Language::Zh),
            "en" => Some(Language::En),
            "ru" => Some(Language::Ru),
            "nl" => Some(Language::Nl),
            _ => None,
        }
    }

    /// Name of the language in that language (shown in the selector).
    pub fn native_name(self) -> &'static str {
        match self {
            Language::Zh => "简体中文",
            Language::En => "English",
            Language::Ru => "Русский",
            Language::Nl => "Nederlands",
        }
    }

    fn index(self) -> u8 {
        match self {
            Language::Zh => 0,
            Language::En => 1,
            Language::Ru => 2,
            Language::Nl => 3,
        }
    }

    fn from_index(index: u8) -> Self {
        match index {
            1 => Language::En,
            2 => Language::Ru,
            3 => Language::Nl,
            _ => Language::Zh,
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.native_name())
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Switch the interface language for the whole process.
pub fn set_language(language: Language) {
    CURRENT.store(language.index(), Ordering::Relaxed);
}

/// Active interface language.
pub fn language() -> Language {
    Language::from_index(CURRENT.load(Ordering::Relaxed))
}

/// Translate `input` into the active language; unchanged when no entry exists.
pub fn tr(input: &str) -> Cow<'_, str> {
    match translate(input) {
        Some(text) => Cow::Owned(text),
        None => Cow::Borrowed(input),
    }
}

/// Translation of `input`, or `None` when the text should be shown as is.
pub fn translate(input: &str) -> Option<String> {
    let lang = language();
    if lang == Language::Zh || !has_cjk(input) {
        return None;
    }
    let index = index();
    if let Some(&row) = index.exact.get(input) {
        return Some(column(row, lang).to_string());
    }
    let cache_key = (lang.index(), input.to_string());
    if let Ok(cache) = cache().lock() {
        if let Some(hit) = cache.get(&cache_key) {
            return hit.clone();
        }
    }
    let result = translate_uncached(input, lang, index);
    if let Ok(mut cache) = cache().lock() {
        if cache.len() > 8192 {
            cache.clear();
        }
        cache.insert(cache_key, result.clone());
    }
    result
}

fn translate_uncached(input: &str, lang: Language, index: &Index) -> Option<String> {
    for template in &index.templates {
        let mut captures = Vec::new();
        if match_segments(&template.key, input, &mut captures) {
            let target = &template.targets[(lang.index() - 1) as usize];
            return Some(fill(&template.key, target, &captures));
        }
    }
    for (separator, replacement) in [("\n", "\n"), (" · ", " · "), ("；", "; ")] {
        if input.contains(separator) {
            let mut changed = false;
            let parts: Vec<String> = input
                .split(separator)
                .map(|part| match translate(part) {
                    Some(text) => {
                        changed = true;
                        text
                    }
                    None => part.to_string(),
                })
                .collect();
            if changed {
                return Some(parts.join(replacement));
            }
        }
    }
    None
}

fn column(row: usize, lang: Language) -> &'static str {
    let entry = &dict::ENTRIES[row];
    let text = match lang {
        Language::Zh => entry.0,
        Language::En => entry.1,
        Language::Ru => entry.2,
        Language::Nl => entry.3,
    };
    if text.is_empty() {
        entry.0
    } else {
        text
    }
}

fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}')
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Hole(String),
}

struct Template {
    key: Vec<Segment>,
    /// en, ru, nl
    targets: [Vec<Segment>; 3],
    literal_len: usize,
}

struct Index {
    exact: HashMap<&'static str, usize>,
    templates: Vec<Template>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut templates = Vec::new();
        for (row, entry) in dict::ENTRIES.iter().enumerate() {
            exact.entry(entry.0).or_insert(row);
            let key = parse(entry.0);
            if !key.iter().any(|s| matches!(s, Segment::Hole(_))) {
                continue;
            }
            let literal_len = key
                .iter()
                .map(|s| match s {
                    Segment::Literal(l) => l.chars().count(),
                    Segment::Hole(_) => 0,
                })
                .sum();
            let target = |text: &'static str| parse(if text.is_empty() { entry.0 } else { text });
            templates.push(Template {
                key,
                targets: [target(entry.1), target(entry.2), target(entry.3)],
                literal_len,
            });
        }
        // Most specific templates first.
        templates.sort_by(|a, b| b.literal_len.cmp(&a.literal_len));
        Index { exact, templates }
    })
}

fn cache() -> &'static Mutex<HashMap<(u8, String), Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<(u8, String), Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Split a `format!` string into literal text and `{…}` placeholders.
fn parse(template: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '{' => {
                let mut token = String::from("{");
                let mut closed = false;
                for d in chars.by_ref() {
                    token.push(d);
                    if d == '}' {
                        closed = true;
                        break;
                    }
                }
                if closed {
                    if !literal.is_empty() {
                        segments.push(Segment::Literal(std::mem::take(&mut literal)));
                    }
                    segments.push(Segment::Hole(token));
                } else {
                    literal.push_str(&token);
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                }
                literal.push('}');
            }
            _ => literal.push(c),
        }
    }
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    segments
}

fn match_segments<'s>(segments: &[Segment], input: &'s str, captures: &mut Vec<&'s str>) -> bool {
    match segments.split_first() {
        None => input.is_empty(),
        Some((Segment::Literal(literal), rest)) => input
            .strip_prefix(literal.as_str())
            .is_some_and(|remaining| match_segments(rest, remaining, captures)),
        Some((Segment::Hole(_), rest)) => match rest.first() {
            None => {
                captures.push(input);
                true
            }
            Some(Segment::Literal(next)) => {
                let mut start = 0;
                while let Some(found) = input[start..].find(next.as_str()) {
                    let at = start + found;
                    captures.push(&input[..at]);
                    if match_segments(rest, &input[at..], captures) {
                        return true;
                    }
                    captures.pop();
                    start = at + next.chars().next().map_or(1, char::len_utf8);
                }
                false
            }
            Some(Segment::Hole(_)) => {
                let ends = input
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(input.len()));
                for end in ends {
                    captures.push(&input[..end]);
                    if match_segments(rest, &input[end..], captures) {
                        return true;
                    }
                    captures.pop();
                }
                false
            }
        },
    }
}

fn is_positional(token: &str) -> bool {
    token == "{}" || token.starts_with("{:")
}

fn fill(key: &[Segment], target: &[Segment], captures: &[&str]) -> String {
    let holes: Vec<&str> = key
        .iter()
        .filter_map(|s| match s {
            Segment::Hole(token) => Some(token.as_str()),
            Segment::Literal(_) => None,
        })
        .collect();
    let mut positional = 0usize;
    let mut out = String::new();
    for segment in target {
        match segment {
            Segment::Literal(literal) => out.push_str(literal),
            Segment::Hole(token) => {
                let slot = if is_positional(token) {
                    let slot = holes
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| is_positional(t))
                        .nth(positional)
                        .map(|(i, _)| i);
                    positional += 1;
                    slot
                } else {
                    holes.iter().position(|t| t == token)
                };
                if let Some(value) = slot.and_then(|i| captures.get(i)) {
                    out.push_str(&tr(value));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test touches the global language so parallel tests cannot race.
    #[test]
    fn translates_exact_templates_and_parts() {
        set_language(Language::Zh);
        assert_eq!(tr("设置"), "设置");

        set_language(Language::En);
        assert_eq!(tr("设置"), "Settings");
        assert_eq!(tr("保存失败：磁盘已满"), "Save failed: 磁盘已满");
        assert_eq!(
            tr("当前：每次询问。适用于关闭按钮和 Alt+F4。"),
            "Current: Ask every time. Applies to the close button and Alt+F4."
        );
        assert_eq!(tr("设置\n审计"), "Settings\nAudit");
        assert_eq!(tr("no chinese here"), "no chinese here");

        set_language(Language::Ru);
        assert_eq!(tr("设置"), "Настройки");
        assert_eq!(tr("俄罗斯"), "Россия");
        set_language(Language::Nl);
        assert_eq!(tr("荷兰"), "Nederland");
        set_language(Language::Zh);
    }

    #[test]
    fn dictionary_placeholders_match_their_keys() {
        for entry in dict::ENTRIES {
            let key: Vec<Segment> = parse(entry.0);
            let mut key_holes: Vec<String> = key
                .iter()
                .filter_map(|s| match s {
                    Segment::Hole(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            key_holes.sort();
            for text in [entry.1, entry.2, entry.3] {
                if text.is_empty() {
                    continue;
                }
                let mut holes: Vec<String> = parse(text)
                    .iter()
                    .filter_map(|s| match s {
                        Segment::Hole(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
                holes.sort();
                assert_eq!(
                    holes, key_holes,
                    "placeholders differ for {:?} -> {:?}",
                    entry.0, text
                );
            }
        }
    }

    #[test]
    fn language_codes_round_trip() {
        for lang in Language::ALL {
            assert_eq!(Language::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Language::from_code("ru-RU"), Some(Language::Ru));
        assert_eq!(Language::from_code("nl-BE"), Some(Language::Nl));
        assert_eq!(Language::from_code("de-DE"), None);
    }
}
