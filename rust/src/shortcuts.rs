//! Validated keyboard preferences; text editors keep their own editing shortcuts.
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Import,
    Palette,
    Save,
    Approve,
    NextIssue,
    Defer,
    Undo,
    Redo,
}
pub const ACTIONS: [(Action, &str); 8] = [
    (Action::Import, "Import scans"),
    (Action::Palette, "Command palette"),
    (Action::Save, "Save draft"),
    (Action::Approve, "Approve page"),
    (Action::NextIssue, "Next issue"),
    (Action::Defer, "Defer issue"),
    (Action::Undo, "Undo review revision"),
    (Action::Redo, "Redo review revision"),
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bindings {
    pub import: String,
    pub palette: String,
    pub save: String,
    pub approve: String,
    pub next_issue: String,
    pub defer: String,
    pub undo: String,
    pub redo: String,
}
impl Default for Bindings {
    fn default() -> Self {
        Self {
            import: "Ctrl+O".into(),
            palette: "Ctrl+K".into(),
            save: "Ctrl+S".into(),
            approve: "Ctrl+Enter".into(),
            next_issue: "Ctrl+J".into(),
            defer: "Ctrl+D".into(),
            undo: "Ctrl+Z".into(),
            redo: "Ctrl+Y".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub primary: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG102", message).into()
}
pub fn parse(value: &str) -> Result<Chord> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 40 && value.is_ascii(),
        fail("Shortcut must be a bounded ASCII key combination")
    );
    let mut chord = Chord {
        primary: false,
        alt: false,
        shift: false,
        key: String::new(),
    };
    let mut tokens = BTreeSet::new();
    for token in value.split('+').map(|s| s.trim().to_ascii_uppercase()) {
        anyhow::ensure!(
            !token.is_empty() && tokens.insert(token.clone()),
            fail("Shortcut contains an empty or repeated modifier/key")
        );
        match token.as_str() {
            "CTRL" | "COMMAND" => {
                anyhow::ensure!(
                    !chord.primary,
                    fail("Ctrl and Command name the same primary modifier")
                );
                chord.primary = true;
            }
            "ALT" => chord.alt = true,
            "SHIFT" => chord.shift = true,
            _ => {
                anyhow::ensure!(
                    chord.key.is_empty(),
                    fail("Choose exactly one shortcut key")
                );
                chord.key = token;
            }
        }
    }
    let function = chord
        .key
        .strip_prefix('F')
        .and_then(|s| s.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n) && chord.key == format!("F{n}"));
    let simple = chord.key.len() == 1 && chord.key.as_bytes()[0].is_ascii_alphanumeric();
    anyhow::ensure!(
        simple || function || ["ENTER", "SPACE", "ESCAPE"].contains(&chord.key.as_str()),
        fail("Choose A–Z, 0–9, Enter, Space, Escape or F1–F12")
    );
    anyhow::ensure!(
        (chord.primary || chord.alt || function) && !(chord.primary && chord.alt),
        fail("Use Ctrl, Alt or a function key; Ctrl+Alt is reserved for international text input")
    );
    anyhow::ensure!(
        !(chord.alt && chord.key == "F4")
            && !(chord.primary
                && !chord.alt
                && ["C", "X", "V", "A", "W", "Q"].contains(&chord.key.as_str()))
            && !(chord.alt && chord.key == "SPACE"),
        fail("This combination is reserved for text editing or window controls")
    );
    Ok(chord)
}
impl Bindings {
    pub fn get(&self, action: Action) -> &str {
        match action {
            Action::Import => &self.import,
            Action::Palette => &self.palette,
            Action::Save => &self.save,
            Action::Approve => &self.approve,
            Action::NextIssue => &self.next_issue,
            Action::Defer => &self.defer,
            Action::Undo => &self.undo,
            Action::Redo => &self.redo,
        }
    }
    pub fn get_mut(&mut self, action: Action) -> &mut String {
        match action {
            Action::Import => &mut self.import,
            Action::Palette => &mut self.palette,
            Action::Save => &mut self.save,
            Action::Approve => &mut self.approve,
            Action::NextIssue => &mut self.next_issue,
            Action::Defer => &mut self.defer,
            Action::Undo => &mut self.undo,
            Action::Redo => &mut self.redo,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for (action, _) in ACTIONS {
            let chord = parse(self.get(action))?;
            anyhow::ensure!(
                seen.insert((chord.primary, chord.alt, chord.shift, chord.key)),
                fail("Two actions use the same shortcut")
            );
        }
        Ok(())
    }
    #[cfg(feature = "desktop")]
    pub fn egui(&self, action: Action) -> eframe::egui::KeyboardShortcut {
        use eframe::egui::{Key, KeyboardShortcut, Modifiers};
        let chord = parse(self.get(action))
            .unwrap_or_else(|_| parse(Self::default().get(action)).expect("default shortcut"));
        let key = match chord.key.as_str() {
            "ENTER" => Key::Enter,
            "SPACE" => Key::Space,
            "ESCAPE" => Key::Escape,
            key => Key::from_name(key).expect("validated shortcut key"),
        };
        KeyboardShortcut::new(
            Modifiers {
                command: chord.primary,
                alt: chord.alt,
                shift: chord.shift,
                ..Default::default()
            },
            key,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_get_complete_defaults_and_conflicts_are_semantic() {
        let defaults: Bindings = serde_json::from_str("{}").unwrap();
        defaults.validate().unwrap();
        let mut bindings = defaults.clone();
        bindings.save = " command + k ".into();
        assert!(bindings.validate().is_err());
        bindings.save = "Ctrl+Shift+S".into();
        bindings.validate().unwrap();
        assert!(serde_json::from_str::<Bindings>(r#"{"unknown":"Ctrl+R"}"#).is_err());
    }
    #[test]
    fn unsafe_text_and_window_combinations_and_malformed_chords_are_rejected() {
        for value in [
            "S",
            "Shift+S",
            "Ctrl+Alt+S",
            "Alt+F4",
            "Alt+Space",
            "Ctrl+V",
            "Ctrl+C",
            "Ctrl+Ctrl+S",
            "Ctrl+Command+S",
            "Ctrl++S",
            "Ctrl+F13",
            "Ctrl+S+J",
            "Ctrl+ä",
            "Ctrl+",
            "F01",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
        for value in ["Ctrl+S", "Command+Shift+Enter", "Alt+J", "F6", "Ctrl+F12"] {
            parse(value).unwrap();
        }
    }
    #[cfg(feature = "desktop")]
    #[test]
    fn every_accepted_key_has_an_actual_native_binding() {
        let mut bindings = Bindings::default();
        for key in ('A'..='Z')
            .chain('0'..='9')
            .map(|c| c.to_string())
            .chain((1..=12).map(|n| format!("F{n}")))
            .chain(["Enter", "Space", "Escape"].map(str::to_owned))
        {
            bindings.save = format!("Alt+Shift+{key}");
            if parse(&bindings.save).is_ok() {
                let _ = bindings.egui(Action::Save);
            }
        }
    }
}
