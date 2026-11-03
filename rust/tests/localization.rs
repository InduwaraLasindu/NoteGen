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

#[test]
fn catalog_templates_reorder_ui_words_but_preserve_opaque_paths_documents_tokens_and_braces() {
    validate_catalog().unwrap();
    let name = "Save {password} sin+eng NG100: not a message".to_owned();
    let path = "C:\\Users\\Private\\passphrase-{secret}\\Qwen3.5-2B.gguf".to_owned();
    let args = [("name", name.clone()), ("path", path.clone())];
    assert_eq!(
        render(Locale::En, "job.saved", &args).unwrap(),
        format!("Saved {name} to {path}")
    );
    assert_eq!(
        render(Locale::Si, "job.saved", &args).unwrap(),
        format!("{path} වෙත {name} සුරකින ලදි.")
    );
    assert_eq!(
        render(Locale::Ta, "job.saved", &args).unwrap(),
        format!("{name} {path} இல் சேமிக்கப்பட்டது.")
    );
    assert_ne!(text(Locale::Si, "save"), "Save");
    assert_eq!(name, "Save {password} sin+eng NG100: not a message");
    assert_eq!(
        render(Locale::En, "JSON {{literal braces}}", &[]).unwrap(),
        "JSON {literal braces}"
    );
    assert_eq!(error_code("NG682: Private path retained"), Some("NG682"));
    assert_eq!(error_code("Document content mentions NG682"), None);
    assert_eq!(error_code("NG6820 is document data"), None);
    assert_eq!(error_code("தமிழ்"), None);
}

#[test]
fn template_argument_mistakes_and_resource_bounds_fail_without_echoing_private_values() {
    for args in [
        vec![],
        vec![("name", "secret".into())],
        vec![("name", "secret".into()), ("name", "second".into())],
        vec![
            ("name", "secret".into()),
            ("path", "private".into()),
            ("unexpected", "other".into()),
        ],
        vec![("name", "secret".into()), ("path", "x".repeat(65_537))],
    ] {
        let error = render(Locale::Si, "job.saved", &args)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"));
        assert!(!error.contains("private"));
    }
    for invalid in [
        "{unterminated",
        "unmatched }",
        "{argument.with.field}",
        "{nested{value}}",
    ] {
        assert!(template_signature(invalid).is_err());
    }
    let unknown = remedy(Locale::Ta, "invalid/document/identifier");
    assert_eq!(unknown.code, "unknown");
    assert_ne!(unknown.title, remedy(Locale::En, "unknown").title);
}

#[test]
fn every_shipped_settings_description_and_recovery_code_has_localized_guidance() {
    validate_catalog().unwrap();
    let guide_keys = notegen::settings_help::GUIDE
        .iter()
        .map(|(key, _)| format!("help.{key}"))
        .collect::<std::collections::BTreeSet<_>>();
    let catalog_help = catalog_keys()
        .into_iter()
        .filter(|key| key.starts_with("help."))
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(catalog_help, guide_keys, "Stale or missing settings help");
    for (key, english) in notegen::settings_help::GUIDE {
        assert_eq!(
            notegen::settings_help::description_localized(Locale::En, key),
            *english
        );
        for locale in [Locale::Si, Locale::Ta] {
            let translated = notegen::settings_help::description_localized(locale, key);
            assert!(!translated.is_empty());
            assert_ne!(translated, *english, "Untranslated {key}");
        }
    }
    for code in [
        "NG100", "NG102", "NG110", "NG200", "NG201", "NG202", "NG203", "NG300", "NG400", "NG401",
        "NG500", "NG610", "NG620", "NG630", "NG640", "NG650", "NG660", "NG661", "NG670", "NG680",
        "NG682", "NG710", "NG720", "NG730", "NG740", "NG750", "NG900", "NG901",
    ] {
        let english = remedy(Locale::En, code);
        for locale in [Locale::Si, Locale::Ta] {
            let translated = remedy(locale, code);
            assert_eq!(translated.code, code);
            assert_ne!(translated.title, english.title, "Untranslated {code}");
            assert!(!translated.checks.is_empty() && !translated.actions.is_empty());
            assert_ne!(translated.checks, english.checks);
            assert_ne!(translated.actions, english.actions);
        }
    }
    assert!(remedy(Locale::En, "NG661").title.contains("activation"));
    assert!(remedy(Locale::En, "NG901").actions[0].contains("automatically"));
    assert!(coverage_note().contains("specialist controls remain in English"));
}

#[cfg(feature = "desktop")]
#[test]
fn independent_ui_contexts_keep_their_own_locale_without_mutating_settings_or_other_contexts() {
    let first = eframe::egui::Context::default();
    let second = eframe::egui::Context::default();
    set_context_locale(&first, Locale::Si);
    set_context_locale(&second, Locale::Ta);
    assert_eq!(context(&first, "save"), "සුරකින්න");
    assert_eq!(context(&second, "save"), "சேமி");
    set_context_locale(&first, Locale::En);
    assert_eq!(context(&first, "save"), "Save");
    assert_eq!(context(&second, "save"), "சேமி");
    assert_eq!(
        context_locale(&eframe::egui::Context::default()),
        Locale::En
    );
}
