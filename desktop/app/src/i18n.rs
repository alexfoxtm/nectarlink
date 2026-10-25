// SPDX-License-Identifier: GPL-3.0-or-later
//! The app's language: the system's or the one chosen in Settings, for the
//! QML (Qt's translators) and for the text the app builds in Rust, through
//! [`tr!`]. `cargo xtask strings` collects every `tr!` text into
//! `desktop/app/i18n/rust_strings.h`, which `lupdate` reads with the QML, so
//! translators see them in the same `.ts` catalogue.

use std::path::{Path, PathBuf};

/// Languages the app speaks, as Settings offers them: a locale name ("" for
/// the system's) and how the language calls itself.
pub const LANGUAGES: &[(&str, &str)] = &[("", ""), ("en", "English")];

/// [`LANGUAGES`] without the system's, as JSON for Settings: `[{ code, name }]`.
pub fn languages_json() -> String {
    let list: Vec<_> = LANGUAGES
        .iter()
        .filter(|(code, _)| !code.is_empty())
        .map(|(code, name)| serde_json::json!({ "code": code, "name": name }))
        .collect();
    serde_json::Value::Array(list).to_string()
}

/// Whether `language` is one [`LANGUAGES`] offers.
pub fn is_known(language: &str) -> bool {
    LANGUAGES.iter().any(|(code, _)| *code == language)
}

// Normalize the language used for display without rewriting a saved preference.
// Retired Chinese choices fall back to English even on a different system locale.
fn display_language(language: &str) -> &str {
    if language.split(['-', '_']).next().is_some_and(|code| code.eq_ignore_ascii_case("zh")) {
        "en"
    } else if is_known(language) {
        language
    } else {
        ""
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// Uses `language` ("" for the system's) from now on, retranslating the
/// open windows. Call on the Qt thread, after the `QGuiApplication` exists.
pub fn install(language: &str) {
    let language = display_language(language);
    let Some(dir) = exe_dir() else { return };
    let installed = crate::bridge::native::ffi::install_translator(&dir.to_string_lossy(), language);
    tracing::debug!(language, installed, "translator");
}

/// `text` (from `context`) in the app's language. Use [`tr!`], which also
/// fills in `%1`, `%2`... and marks the text for `cargo xtask strings`.
/// With no translation installed (or none for `text`), `text` itself.
pub fn translate(context: &str, text: &str) -> String {
    crate::bridge::native::ffi::translate(context, text)
}

/// Puts `args` in for `%1`, `%2`... in `text` (Qt's placeholders, which
/// translators may reorder). `%%` is a literal `%`.
pub fn fill(text: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some(d @ '1'..='9') => {
                chars.next();
                match args.get(d as usize - '1' as usize) {
                    Some(arg) => out.push_str(&arg.to_string()),
                    None => {
                        out.push('%');
                        out.push(d);
                    }
                }
            }
            _ => out.push('%'),
        }
    }
    out
}

/// Text the app shows, in the user's language: `tr!("Context", "Sent to %1", name)`.
/// The context and text must be literals (`cargo xtask strings` reads them).
macro_rules! tr {
    ($context:literal, $text:literal $(,)?) => {
        $crate::i18n::translate($context, $text)
    };
    ($context:literal, $text:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::fill(&$crate::i18n::translate($context, $text), &[$(&$arg as &dyn std::fmt::Display),+])
    };
}
pub(crate) use tr;

/// A session's title in the app's language, by what it was (the core's own
/// title is English).
pub fn session_title(kind: nectarlink_core::SessionKind) -> String {
    use nectarlink_core::SessionKind;
    match kind {
        SessionKind::PhoneScreen | SessionKind::PcScreen => tr!("Timeline", "Screen mirroring"),
        SessionKind::AppWindow(n) => tr!("Timeline", "App window #%1", n),
        SessionKind::Webcam => tr!("Timeline", "Phone webcam"),
        SessionKind::Microphone => tr!("Timeline", "Phone microphone"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_fill_in_any_order() {
        assert_eq!(fill("Sent to %1", &[&"Pixel"]), "Sent to Pixel");
        assert_eq!(fill("%2 then %1", &[&"a", &"b"]), "b then a");
        assert_eq!(fill("100%% of %1", &[&3]), "100% of 3");
        // A placeholder without an argument stays, so the mistake shows.
        assert_eq!(fill("%1 and %2", &[&"x"]), "x and %2");
        assert_eq!(fill("50% off", &[]), "50% off");
    }

    #[test]
    fn tr_fills_its_arguments() {
        let name = "Pixel 9";
        assert_eq!(tr!("Tests", "Sent to %1", name), "Sent to Pixel 9");
        assert_eq!(tr!("Tests", "Done"), "Done");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bundled_catalog_loads_from_resources_and_missing_locale_falls_back() {
        let _app = cxx_qt_lib::QCoreApplication::new();
        let scratch = tempfile::tempdir().unwrap();
        // A data-only directory layout, without an app bundle or registration.
        let contents = scratch.path().join("translation-fixture/Contents");
        let executable_dir = contents.join("MacOS");
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&executable_dir).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            resources.join("nectarlink_de_DE.qm"),
            include_bytes!("mac/testdata/i18n-fixture-de.qm"),
        )
        .unwrap();

        let dir = executable_dir.to_string_lossy();
        assert!(crate::bridge::native::ffi::install_translator(&dir, "de_DE"));
        assert_eq!(translate("WaypairI18nFixture", "Resource path"), "Pfad");
        assert!(!crate::bridge::native::ffi::install_translator(&dir, "en"));
        assert_eq!(translate("WaypairI18nFixture", "Resource path"), "Resource path");
    }

    #[test]
    fn only_offered_languages_are_kept() {
        assert!(is_known("") && is_known("en"));
        for removed in ["zh", "zh_CN", "zh-CN", "zh_HK", "zh-Hant", "ZH_cn"] {
            assert!(!is_known(removed));
            assert_eq!(display_language(removed), "en");
        }
        assert_eq!(display_language("en"), "en");
        assert_eq!(display_language(""), "");
        assert_eq!(display_language("xx"), "");
        let listed: Vec<serde_json::Value> = serde_json::from_str(&languages_json()).unwrap();
        assert_eq!(listed, vec![serde_json::json!({ "code": "en", "name": "English" })]);
        assert!(listed.iter().all(|l| l["code"] != ""), "the system's isn't a choice in the list");
        assert!(!is_known("xx"));
    }

    /// A language Settings offers is fully translated: languages still being
    /// translated (docs/translating.md) stay out of [`LANGUAGES`] until they're done.
    #[test]
    fn every_offered_language_is_fully_translated() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n");
        for (code, name) in LANGUAGES.iter().filter(|(code, _)| !code.is_empty() && *code != "en") {
            let ts = std::fs::read_to_string(dir.join(format!("nectarlink_{code}.ts")))
                .unwrap_or_else(|e| panic!("{name} is offered but has no catalogue: {e}"));
            let unfinished = ts.matches("type=\"unfinished\"").count();
            assert_eq!(unfinished, 0, "{name} is offered but has {unfinished} untranslated strings");
        }
    }
}
