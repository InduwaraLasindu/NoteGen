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
    let mut s = Settings::default();
    s.ui.locale = Locale::Ta;
    s.languages = "sin+eng".into();
    let restored: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(restored.ui.locale, Locale::Ta);
    assert_eq!(restored.languages, "sin+eng");
    assert!(coverage_note().contains("Detailed help"));
}
#[test]
fn old_settings_without_interface_locale_or_custom_tessdata_still_load() {
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    old.as_object_mut().unwrap().remove("tessdata_dir");
    old["ui"].as_object_mut().unwrap().remove("locale");
    let restored: Settings = serde_json::from_value(old).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.ui.locale, Locale::En);
    assert!(restored.tessdata_dir.as_os_str().is_empty());
    assert_eq!(restored.languages, "eng");
}
