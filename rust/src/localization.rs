//! Core interface labels. OCR language and interface language are independent settings.
use serde::{Deserialize, Serialize};

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

/// Unknown keys fall back to their readable English caller label. Detailed help remains English.
pub fn text(locale: Locale, key: &str) -> &str {
    let labels = match key {
        "import_scans" => ["Import scans", "ස්කෑන් ආයාත කරන්න", "ஸ்கேன்களை இறக்குமதி செய்"],
        "start_conversion" => ["Start conversion", "පරිවර්තනය අරඹන්න", "மாற்றத்தைத் தொடங்கு"],
        "pause" => ["Pause", "විරාම කරන්න", "இடைநிறுத்து"],
        "resume" => ["Resume", "නැවත අරඹන්න", "தொடர்"],
        "cancel" => ["Cancel", "අවලංගු කරන්න", "ரத்துசெய்"],
        "settings" => ["Settings", "සැකසුම්", "அமைப்புகள்"],
        "advanced" => ["Advanced", "උසස් සැකසුම්", "மேம்பட்ட அமைப்புகள்"],
        "projects_tools" => ["Projects & tools", "ව්‍යාපෘති සහ මෙවලම්", "திட்டங்களும் கருவிகளும்"],
        "review" => ["Review", "සමාලෝචනය", "மதிப்பாய்வு"],
        "export" => ["Export", "නිර්යාත කරන්න", "ஏற்றுமதி செய்"],
        "close" => ["Close", "වසන්න", "மூடு"],
        "save" => ["Save", "සුරකින්න", "சேமி"],
        "search" => ["Search", "සොයන්න", "தேடு"],
        "one_page_trial" => [
            "One-page trial",
            "එක් පිටුවක් අත්හදා බලන්න",
            "ஒரு பக்கத்தைச் சோதிக்கவும்",
        ],
        "queue" => ["Queue", "පෝලිම", "வரிசை"],
        "overall_progress" => ["Overall progress", "සමස්ත ප්‍රගතිය", "மொத்த முன்னேற்றம்"],
        "install_ai" => ["Install AI", "AI ස්ථාපනය කරන්න", "AI நிறுவு"],
        "output_folder" => ["Output folder", "ප්‍රතිදාන ෆෝල්ඩරය", "வெளியீட்டுக் கோப்புறை"],
        "source" => ["Source", "මුල් පිටපත", "மூலம்"],
        "interface_language" => ["Interface language", "අතුරු මුහුණතේ භාෂාව", "இடைமுக மொழி"],
        "ocr_language" => ["Recognition language", "හඳුනාගැනීමේ භාෂාව", "எழுத்துணர்வு மொழி"],
        "undo" => ["Undo", "අහෝසි කරන්න", "செயல்தவிர்"],
        "redo" => ["Redo", "නැවත කරන්න", "மீண்டும் செய்"],
        "approve" => ["Approve", "අනුමත කරන්න", "ஒப்புதல் அளி"],
        "help" => ["Help", "උදව්", "உதவி"],
        _ => return key,
    };
    labels[match locale {
        Locale::En => 0,
        Locale::Si => 1,
        Locale::Ta => 2,
    }]
}
pub fn coverage_note() -> &'static str {
    "Core action labels are available in English, Sinhala and Tamil. Detailed help, error explanations and specialist tools still use English. Translation and complex-script rendering need native-speaker review; UI locale does not change OCR languages."
}
