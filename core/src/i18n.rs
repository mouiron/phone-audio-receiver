//! Windowsの優先表示言語を基に、GUIの表示言語を選択する。

use windows::core::PWSTR;
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Japanese,
    English,
}

impl Language {
    pub fn select(self, japanese: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::Japanese => japanese,
            Self::English => english,
        }
    }
}

pub fn system_language() -> Language {
    let mut count = 0;
    let mut chars = 0;
    let result =
        unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut chars) };
    if result.is_err() || chars == 0 {
        return Language::English;
    }

    let mut buffer = vec![0u16; chars as usize];
    let result = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut chars,
        )
    };
    if result.is_err() {
        return Language::English;
    }

    language_from_tags(&String::from_utf16_lossy(&buffer))
}

fn language_from_tags(preferred: &str) -> Language {
    let primary = preferred.split('\0').next().unwrap_or_default();
    if primary.eq_ignore_ascii_case("ja") || primary.to_ascii_lowercase().starts_with("ja-") {
        Language::Japanese
    } else {
        Language::English
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_is_selected_when_it_is_the_primary_display_language() {
        assert_eq!(language_from_tags("ja-JP\0en-US\0\0"), Language::Japanese);
    }

    #[test]
    fn english_is_selected_when_japanese_is_absent() {
        assert_eq!(language_from_tags("en-US\0de-DE\0\0"), Language::English);
    }

    #[test]
    fn primary_language_takes_precedence_over_secondary_languages() {
        assert_eq!(language_from_tags("en-US\0ja-JP\0\0"), Language::English);
    }
}
