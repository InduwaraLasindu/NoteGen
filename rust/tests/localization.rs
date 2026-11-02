use notegen::{localization::*, types::Settings};
#[test]
fn core_keys_have_distinct_nonempty_translations_in_each_locale() {
    for key in KEYS {
        let en = text(Locale::En, key);
        assert!(!en.is_empty());
        assert_ne!(en, *key);
        for locale in [Locale::Si, Locale::Ta] {
            let translated = text(locale, key);
            assert!(!translated.is_empty());
            assert_ne!(translated, en);
        }
    }
}
#[test]
fn locale_codes_round_trip_and_unknown_labels_fall_back() {
    for locale in locales() {
        let encoded = serde_json::to_string(&locale).unwrap();
        assert_eq!(serde_json::from_str::<Locale>(&encoded).unwrap(), locale);
        assert_eq!(encoded, format!("\"{}\"", locale.code()));
        assert!(!locale.label().is_empty());
    }
    assert_eq!(Locale::default(), Locale::En);
    assert!(serde_json::from_str::<Locale>("\"unknown\"").is_err());
    assert_eq!(text(Locale::Si, "Untranslated help"), "Untranslated help");
}
#[test]
fn interface_translation_does_not_modify_ocr_language_policy() {
    let s = Settings::default();
    for locale in locales() {
        let _ = text(locale, "ocr_language");
        assert_eq!(s.languages, "eng");
    }
    assert!(coverage_note().contains("Detailed help"));
}
