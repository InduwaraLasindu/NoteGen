//! Packaged interface catalogs. Document, secret, path and protocol values are opaque arguments.
use crate::types::{Fault, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    #[default]
    En,
    Si,
    Ta,
}
impl Locale {
    pub fn label(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Si => "සිංහල",
            Self::Ta => "தமிழ்",
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Si => "si",
            Self::Ta => "ta",
        }
    }
}
pub fn locales() -> [Locale; 3] {
    [Locale::En, Locale::Si, Locale::Ta]
}
pub const KEYS: &[&str] = &[
    "import_scans",
    "start_conversion",
    "pause",
    "resume",
    "cancel",
    "settings",
    "advanced",
    "projects_tools",
    "review",
    "export",
    "close",
    "save",
    "search",
    "one_page_trial",
    "queue",
    "overall_progress",
    "install_ai",
    "output_folder",
    "source",
    "interface_language",
    "ocr_language",
    "undo",
    "redo",
    "approve",
    "help",
];

const MAX_CATALOG_BYTES: usize = 8_000_000;
const MAX_MESSAGE_BYTES: usize = 16_384;
const MAX_ARGUMENT_BYTES: usize = 65_536;
const MAX_OUTPUT_BYTES: usize = 262_144;
struct Catalog {
    en: BTreeMap<String, String>,
    si: BTreeMap<String, String>,
    ta: BTreeMap<String, String>,
    source_keys: BTreeMap<String, String>,
    templates: BTreeSet<String>,
}
static CATALOG: OnceLock<std::result::Result<Catalog, String>> = OnceLock::new();
fn invalid() -> anyhow::Error {
    Fault::new(
        "NG102",
        "Interface catalog or message placeholders are invalid",
    )
    .into()
}
fn unique_map(bytes: &str) -> Result<BTreeMap<String, String>> {
    use serde::de::{MapAccess, Visitor};
    struct Unique;
    impl<'de> Visitor<'de> for Unique {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded message catalog with unique keys")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if out.len() >= 10_000
                    || key.is_empty()
                    || key.len() > 160
                    || !key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    || value.is_empty()
                    || value.len() > MAX_MESSAGE_BYTES
                    || out.insert(key, value).is_some()
                {
                    return Err(serde::de::Error::custom(
                        "invalid, duplicate or oversized interface message",
                    ));
                }
            }
            Ok(out)
        }
    }
    anyhow::ensure!(bytes.len() <= MAX_CATALOG_BYTES, invalid());
    let mut parser = serde_json::Deserializer::from_str(bytes);
    let result = serde::de::Deserializer::deserialize_map(&mut parser, Unique)?;
    parser.end()?;
    Ok(result)
}
enum Part {
    Literal(String),
    Argument(String),
}
fn parts(template: &str) -> Result<Vec<Part>> {
    anyhow::ensure!(template.len() <= MAX_MESSAGE_BYTES, invalid());
    let mut out = vec![];
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    let mut placeholders = 0;
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    let _ = chars.next();
                    literal.push('{');
                    continue;
                }
                if !literal.is_empty() {
                    out.push(Part::Literal(std::mem::take(&mut literal)));
                }
                let mut name = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    anyhow::ensure!(
                        name.len() < 64 && (c.is_ascii_alphanumeric() || c == '_'),
                        invalid()
                    );
                    name.push(c);
                }
                placeholders += 1;
                anyhow::ensure!(closed && !name.is_empty() && placeholders <= 128, invalid());
                out.push(Part::Argument(name));
            }
            '}' => {
                anyhow::ensure!(chars.next() == Some('}'), invalid());
                literal.push('}');
            }
            _ => literal.push(ch),
        }
    }
    if !literal.is_empty() {
        out.push(Part::Literal(literal));
    }
    Ok(out)
}
pub fn template_signature(template: &str) -> Result<BTreeMap<String, usize>> {
    let mut signature = BTreeMap::new();
    for part in parts(template)? {
        if let Part::Argument(name) = part {
            *signature.entry(name).or_default() += 1;
        }
    }
    Ok(signature)
}
fn load_catalog() -> Result<Catalog> {
    let en = unique_map(include_str!("../i18n/en.json"))?;
    let si = unique_map(include_str!("../i18n/si.json"))?;
    let ta = unique_map(include_str!("../i18n/ta.json"))?;
    anyhow::ensure!(
        !en.is_empty()
            && en.keys().eq(si.keys())
            && en.keys().eq(ta.keys())
            && KEYS.iter().all(|key| en.contains_key(*key)),
        invalid()
    );
    let list: Vec<String> = serde_json::from_str(include_str!("../i18n/templates.json"))?;
    let templates = list.iter().cloned().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        list.len() == templates.len() && templates.len() <= 10_000,
        invalid()
    );
    for key in &templates {
        let english = en.get(key).ok_or_else(invalid)?;
        let signature = template_signature(english)?;
        anyhow::ensure!(
            signature == template_signature(&si[key])?
                && signature == template_signature(&ta[key])?,
            invalid()
        );
    }
    let mut source_keys = BTreeMap::new();
    for (key, value) in &en {
        source_keys
            .entry(value.clone())
            .or_insert_with(|| key.clone());
    }
    Ok(Catalog {
        en,
        si,
        ta,
        source_keys,
        templates,
    })
}
fn catalog() -> Option<&'static Catalog> {
    CATALOG
        .get_or_init(|| load_catalog().map_err(|e| e.to_string()))
        .as_ref()
        .ok()
}
pub fn validate_catalog() -> Result<()> {
    load_catalog().map(|_| ())
}
pub fn has_key(key: &str) -> bool {
    catalog().is_some_and(|c| c.en.contains_key(key))
}
pub fn catalog_keys() -> Vec<&'static str> {
    catalog().map_or_else(Vec::new, |c| c.en.keys().map(String::as_str).collect())
}
/// Explicit presentation keys, or exact known English UI text. Unknown caller
/// labels stay readable. Never apply this function to recognition or secret data.
pub fn text<'a>(locale: Locale, key: &'a str) -> &'a str {
    let Some(c) = catalog() else {
        return key;
    };
    let key = if c.en.contains_key(key) {
        key
    } else if let Some(id) = c.source_keys.get(key) {
        id.as_str()
    } else {
        return key;
    };
    match locale {
        Locale::En => c.en.get(key),
        Locale::Si => c.si.get(key),
        Locale::Ta => c.ta.get(key),
    }
    .or_else(|| c.en.get(key))
    .map_or(key, String::as_str)
}
pub fn message(locale: Locale, key: &str) -> &str {
    text(locale, key)
}
/// Named values are inserted literally once per declared occurrence. They are
/// never translated or parsed recursively (including braces, passwords or paths).
pub fn render(locale: Locale, key: &str, args: &[(&str, String)]) -> Result<String> {
    anyhow::ensure!(args.len() <= 32, invalid());
    let mut values = BTreeMap::new();
    for (name, value) in args {
        anyhow::ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && value.len() <= MAX_ARGUMENT_BYTES
                && values.insert(*name, value.as_str()).is_none(),
            invalid()
        );
    }
    let template = text(locale, key);
    if let Some(c) = catalog()
        && c.en.contains_key(key)
        && !c.templates.contains(key)
    {
        anyhow::ensure!(args.is_empty(), invalid());
        return Ok(template.to_owned());
    }
    let parsed = parts(template)?;
    let required = parsed
        .iter()
        .filter_map(|p| {
            if let Part::Argument(n) = p {
                Some(n.as_str())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        required.len() == values.len() && required.iter().all(|name| values.contains_key(*name)),
        invalid()
    );
    let mut out = String::new();
    for part in parsed {
        let value = match &part {
            Part::Literal(value) => value.as_str(),
            Part::Argument(name) => values[name.as_str()],
        };
        anyhow::ensure!(
            out.len().saturating_add(value.len()) <= MAX_OUTPUT_BYTES,
            invalid()
        );
        out.push_str(value);
    }
    Ok(out)
}
#[derive(Clone, Debug, Serialize)]
pub struct Remedy {
    pub code: String,
    pub title: String,
    pub checks: Vec<String>,
    pub actions: Vec<String>,
}
pub fn remedy(locale: Locale, code: &str) -> Remedy {
    let valid = code.len() == 5
        && code.starts_with("NG")
        && code.as_bytes()[2..].iter().all(u8::is_ascii_digit);
    let code = if valid { code } else { "unknown" };
    let prefix = format!("error.{code}");
    if has_key(&format!("{prefix}.title")) {
        let collect = |kind: &str| {
            (0..32)
                .map(|i| format!("{prefix}.{kind}.{i}"))
                .take_while(|key| has_key(key))
                .map(|key| text(locale, &key).to_owned())
                .collect()
        };
        return Remedy {
            code: code.into(),
            title: text(locale, &format!("{prefix}.title")).into(),
            checks: collect("check"),
            actions: collect("action"),
        };
    }
    let base = crate::diagnostics::troubleshoot(code);
    Remedy {
        code: base.code,
        title: text(locale, &base.title).into(),
        checks: base.checks.iter().map(|v| text(locale, v).into()).collect(),
        actions: base
            .actions
            .iter()
            .map(|v| text(locale, v).into())
            .collect(),
    }
}
pub fn error_code(details: &str) -> Option<&str> {
    let code = details.get(..5)?;
    if code.starts_with("NG")
        && code.as_bytes()[2..].iter().all(u8::is_ascii_digit)
        && details
            .as_bytes()
            .get(5)
            .is_none_or(|b| b.is_ascii_whitespace() || *b == b':')
    {
        Some(code)
    } else {
        None
    }
}
#[cfg(feature = "desktop")]
pub fn set_context_locale(ctx: &eframe::egui::Context, locale: Locale) {
    ctx.data_mut(|d| d.insert_temp(eframe::egui::Id::unique("notegen-interface-locale"), locale));
}
#[cfg(feature = "desktop")]
pub fn context_locale(ctx: &eframe::egui::Context) -> Locale {
    ctx.data(|d| d.get_temp(eframe::egui::Id::unique("notegen-interface-locale")))
        .unwrap_or_default()
}
#[cfg(feature = "desktop")]
pub fn ui(ui: &eframe::egui::Ui, key: &str) -> String {
    text(context_locale(ui.ctx()), key).to_owned()
}
#[cfg(feature = "desktop")]
pub fn context(ctx: &eframe::egui::Context, key: &str) -> String {
    text(context_locale(ctx), key).to_owned()
}
#[cfg(feature = "desktop")]
pub fn ui_template(ui: &eframe::egui::Ui, key: &str, args: &[(&str, String)]) -> String {
    render(context_locale(ui.ctx()), key, args)
        .unwrap_or_else(|_| text(context_locale(ui.ctx()), "template_error").into())
}
#[cfg(feature = "desktop")]
pub fn context_template(ctx: &eframe::egui::Context, key: &str, args: &[(&str, String)]) -> String {
    render(context_locale(ctx), key, args)
        .unwrap_or_else(|_| text(context_locale(ctx), "template_error").into())
}
pub fn coverage_note() -> &'static str {
    "Interface catalogs are independent of OCR/model languages. Detailed help and error remedies use the same catalogs. Native-speaker translation and visual acceptance remain pending."
}
