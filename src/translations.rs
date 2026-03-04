// Translation management for runtime language switching
//
// Since Slint's @tr() macro doesn't react to runtime language changes,
// we manage translations as Rust-side properties that get updated manually.

use crate::localization::Language;

pub fn translate(key: &str, language: Language) -> String {
    match language {
        Language::English => translate_en(key),
        Language::Swedish => translate_sv(key),
        Language::Arabic => translate_ar(key),
    }
}

fn translate_en(key: &str) -> String {
    match key {
        "Live" => "Live",
        "Podcasts" => "Podcasts",
        "News" => "News",
        "Music" => "Music",
        "Favorites" => "Favorites",
        "Settings" => "Settings",
        "Vol:" => "Vol:",
        "LIVE" => "LIVE",
        "Language:" => "Language:",
        "Browse Podcasts" => "Browse Podcasts",
        "← Back" => "← Back",
        "Loading channels..." => "Loading channels...",
        "Loading programs..." => "Loading programs...",
        "Loading news programs..." => "Loading news programs...",
        "No channels found" => "No channels found",
        "No programs found" => "No programs found",
        "No episodes found" => "No episodes found",
        "No news programs found" => "No news programs found",
        "No favorites yet. Click the star icon on channels or podcasts to add them." =>
            "No favorites yet. Click the star icon on channels or podcasts to add them.",
        "Search podcasts..." => "Search podcasts...",
        "Music content coming soon..." => "Music content coming soon...",
        "Downloading" => "Downloading",
        "Downloaded" => "Downloaded",
        "Copied" => "Copied",
        "Podcast Episodes" => "Podcast Episodes",
        "Connecting..." => "Connecting...",
        "Connection failed" => "Connection failed",
        "Keep channels alive (fast switching)" => "Keep channels alive (fast switching)",
        "Preferences..." => "Preferences...",
        "About SR Player" => "About SR Player",
        _ => key, // Return key if not found
    }.to_string()
}

fn translate_sv(key: &str) -> String {
    match key {
        "Live" => "Live",
        "Podcasts" => "Poddradio",
        "News" => "Nyheter",
        "Music" => "Musik",
        "Favorites" => "Favoriter",
        "Settings" => "Inställningar",
        "Vol:" => "Vol:",
        "LIVE" => "DIREKT",
        "Language:" => "Språk:",
        "Browse Podcasts" => "Bläddra Poddar",
        "← Back" => "← Tillbaka",
        "Loading channels..." => "Laddar kanaler...",
        "Loading programs..." => "Laddar program...",
        "Loading news programs..." => "Laddar nyhetsprogram...",
        "No channels found" => "Inga kanaler hittades",
        "No programs found" => "Inga program hittades",
        "No episodes found" => "Inga avsnitt hittades",
        "No news programs found" => "Inga nyhetsprogram hittades",
        "No favorites yet. Click the star icon on channels or podcasts to add them." =>
            "Inga favoriter ännu. Klicka på stjärnikonen på kanaler eller poddar för att lägga till dem.",
        "Search podcasts..." => "Sök poddar...",
        "Music content coming soon..." => "Musikinnehåll kommer snart...",
        "Downloading" => "Laddar ner",
        "Downloaded" => "Nedladdad",
        "Copied" => "Kopierad",
        "Podcast Episodes" => "Poddavsnitt",
        "Connecting..." => "Ansluter...",
        "Connection failed" => "Anslutningen misslyckades",
        "Keep channels alive (fast switching)" => "Håll kanaler aktiva (snabbt byte)",
        "Preferences..." => "Inställningar...",
        "About SR Player" => "Om SR Player",
        _ => key, // Return key if not found
    }.to_string()
}

fn translate_ar(key: &str) -> String {
    match key {
        "Live" => "مباشر",
        "Podcasts" => "البودكاست",
        "News" => "الأخبار",
        "Music" => "الموسيقى",
        "Favorites" => "المفضلة",
        "Settings" => "الإعدادات",
        "Vol:" => "الصوت:",
        "LIVE" => "مباشر",
        "Language:" => "اللغة:",
        "Browse Podcasts" => "تصفح البودكاست",
        "← Back" => "→ رجوع",  // Arrow reversed for RTL
        "Loading channels..." => "جاري تحميل القنوات...",
        "Loading programs..." => "جاري تحميل البرامج...",
        "Loading news programs..." => "جاري تحميل برامج الأخبار...",
        "No channels found" => "لم يتم العثور على قنوات",
        "No programs found" => "لم يتم العثور على برامج",
        "No episodes found" => "لم يتم العثور على حلقات",
        "No news programs found" => "لم يتم العثور على برامج أخبار",
        "No favorites yet. Click the star icon on channels or podcasts to add them." =>
            "لا توجد مفضلات بعد. انقر على أيقونة النجمة على القنوات أو البودكاست لإضافتها.",
        "Search podcasts..." => "البحث عن بودكاست...",
        "Music content coming soon..." => "محتوى الموسيقى قريباً...",
        "Downloading" => "جاري التحميل",
        "Downloaded" => "تم التحميل",
        "Copied" => "تم النسخ",
        "Podcast Episodes" => "حلقات البودكاست",
        "Connecting..." => "جاري الاتصال...",
        "Connection failed" => "فشل الاتصال",
        "Keep channels alive (fast switching)" => "إبقاء القنوات نشطة (تبديل سريع)",
        "Preferences..." => "التفضيلات...",
        "About SR Player" => "حول SR Player",
        _ => key, // Return key if not found
    }.to_string()
}
