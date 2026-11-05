//! Interface language for the GUI.
//!
//! Views keep their Simplified Chinese source strings; `text` and `text_input`
//! below replace the Iced constructors and translate at render time through
//! `envbox_core::i18n`. The chosen language is stored in `ui.toml`; without a
//! stored choice the Windows display language decides (Chinese, Russian,
//! Dutch, otherwise English).

use std::path::Path;

pub use envbox_core::i18n::{language, set_language, tr, Language};

/// Translating replacement for `iced::widget::text`.
pub fn text<'a, Theme, Renderer>(
    content: impl iced::widget::text::IntoFragment<'a>,
) -> iced::widget::text::Text<'a, Theme, Renderer>
where
    Theme: iced::widget::text::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer,
{
    let fragment = content.into_fragment();
    match envbox_core::i18n::translate(&fragment) {
        Some(translated) => iced::widget::text(translated),
        None => iced::widget::text(fragment),
    }
}

/// Translating replacement for `iced::widget::text_input` (placeholder only;
/// the edited value is never translated).
pub fn text_input<'a, Message, Theme, Renderer>(
    placeholder: &str,
    value: &str,
) -> iced::widget::text_input::TextInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: iced::widget::text_input::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer,
{
    iced::widget::text_input(&tr(placeholder), value)
}

/// Apply the stored language, or the Windows display language on first run.
pub fn init(config_root: &Path) {
    let language = crate::close_behavior::load_language(config_root)
        .and_then(|code| Language::from_code(&code))
        .unwrap_or_else(system_language);
    set_language(language);
}

/// Change the language for this process and remember it.
pub fn select(config_root: &Path, language: Language) -> Result<(), String> {
    set_language(language);
    crate::close_behavior::save_language(config_root, language.code())
}

#[cfg(windows)]
fn system_language() -> Language {
    // Primary language id of the user's Windows display language.
    let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    match lang_id & 0x03ff {
        0x04 => Language::Zh,
        0x19 => Language::Ru,
        0x13 => Language::Nl,
        _ => Language::En,
    }
}

#[cfg(not(windows))]
fn system_language() -> Language {
    std::env::var("LANG")
        .ok()
        .and_then(|value| Language::from_code(&value))
        .unwrap_or(Language::En)
}
