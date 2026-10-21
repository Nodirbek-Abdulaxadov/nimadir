//! The window's own words, in Uzbek, Russian and English — the chrome, the
//! questions it asks, the settings page. Chosen once, from `crate::locale()`.
//!
//! Only what the window *says to a person*. Status lines and the headless
//! runner's output stay in English: logs and the test suite read them, and a
//! diagnostic is better stable than translated.

/// The languages the window speaks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Uz,
    Ru,
    En,
}

impl Lang {
    /// From a BCP 47 tag: `uz-UZ`, `uz-Latn-UZ` and `uz` are Uzbek, and so on;
    /// anything this does not speak is English.
    pub fn from_tag(tag: &str) -> Self {
        match tag.split(['-', '_']).next().unwrap_or("") {
            "uz" => Lang::Uz,
            "ru" => Lang::Ru,
            _ => Lang::En,
        }
    }

    pub fn current() -> Self {
        Lang::from_tag(&crate::locale())
    }
}

/// Everything the window says. Placeholders — `{app}`, `{address}`,
/// `{origin}` — are filled in where the text is used.
#[derive(Clone, Copy)]
pub enum Text {
    Back,
    Home,
    Reload,
    Open,
    Stay,
    Allow,
    DontAllow,
    WantsToOpen,
    WantsToConnect,
    Loading,
    TypeSomething,
    Settings,
    WhatAppsKeep,
    NothingKept,
    ClearData,
    Caches,
    ClearCache,
    CachesRebuild,
    DataDir,
    CacheDir,
    MiniApps,
    NoMiniApps,
    WebPage,
    WebPageRouted,
    // Only said where there is no webview to show the page in.
    #[cfg(not(feature = "webview"))]
    WebPageRebuild,
    #[cfg(not(feature = "webview"))]
    WebPageNotHere,
    Updates,
    CheckForUpdates,
    Checking,
    Newest,
    NewerOut,
}

pub fn t(lang: Lang, text: Text) -> &'static str {
    use Lang::*;
    use Text::*;
    match (text, lang) {
        (Back, Uz) => "Orqaga",
        (Back, Ru) => "Назад",
        (Back, En) => "Back",
        (Home, Uz) => "Bosh sahifa",
        (Home, Ru) => "Домой",
        (Home, En) => "Home",
        (Reload, Uz) => "Yangilash",
        (Reload, Ru) => "Обновить",
        (Reload, En) => "Reload",
        (Open, Uz) => "Ochish",
        (Open, Ru) => "Открыть",
        (Open, En) => "Open",
        (Stay, Uz) => "Qolish",
        (Stay, Ru) => "Остаться",
        (Stay, En) => "Stay",
        (Allow, Uz) => "Ruxsat berish",
        (Allow, Ru) => "Разрешить",
        (Allow, En) => "Allow",
        (DontAllow, Uz) => "Ruxsat bermaslik",
        (DontAllow, Ru) => "Не разрешать",
        (DontAllow, En) => "Don't allow",
        (WantsToOpen, Uz) => "{app} {address} manzilini ochmoqchi",
        (WantsToOpen, Ru) => "{app} хочет открыть {address}",
        (WantsToOpen, En) => "{app} wants to open {address}",
        (WantsToConnect, Uz) => "{app} {origin} bilan bogʻlanmoqchi",
        (WantsToConnect, Ru) => "{app} хочет подключиться к {origin}",
        (WantsToConnect, En) => "{app} wants to connect to {origin}",
        (Loading, Uz) => "Yuklanmoqda: {address}",
        (Loading, Ru) => "Загрузка: {address}",
        (Loading, En) => "Loading {address}",
        (TypeSomething, Uz) => "Ilova nomi, .wasm yoʻli yoki manzil yozing",
        (TypeSomething, Ru) => "Введите имя приложения, путь к .wasm или адрес",
        (TypeSomething, En) => "type a .wasm path, a URL, or the name of a listed app",
        (Settings, Uz) => "Sozlamalar",
        (Settings, Ru) => "Настройки",
        (Settings, En) => "Settings",
        (WhatAppsKeep, Uz) => "Ilovalar saqlagan maʼlumotlar",
        (WhatAppsKeep, Ru) => "Что хранят приложения",
        (WhatAppsKeep, En) => "What apps keep",
        (NothingKept, Uz) => {
            "Hozircha hech narsa. Faqat imzolangan va saqlashni soʻragan ilova maʼlumot saqlaydi."
        }
        (NothingKept, Ru) => {
            "Пока ничего. Хранить данные может только подписанное приложение, которое об этом просит."
        }
        (NothingKept, En) => "Nothing yet. Only a signed app that asks for storage keeps anything.",
        (ClearData, Uz) => "Maʼlumotni oʻchirish",
        (ClearData, Ru) => "Удалить данные",
        (ClearData, En) => "Clear data",
        (Caches, Uz) => "Yuklamalar va kompilyatsiya qilingan kod",
        (Caches, Ru) => "Загрузки и скомпилированный код",
        (Caches, En) => "Downloads and compiled code",
        (ClearCache, Uz) => "Keshni tozalash",
        (ClearCache, Ru) => "Очистить кэш",
        (ClearCache, En) => "Clear cache",
        (CachesRebuild, Uz) => {
            "Hammasini qayta yuklab olish va kompilyatsiya qilish mumkin; keyingi ochilishlar sekinroq boʻladi."
        }
        (CachesRebuild, Ru) => {
            "Всё это можно скачать и скомпилировать заново; следующие открытия будут медленнее."
        }
        (CachesRebuild, En) => {
            "All of it can be fetched or compiled again; the next opens are slower."
        }
        (DataDir, Uz) => "Maʼlumotlar",
        (DataDir, Ru) => "Данные",
        (DataDir, En) => "Data",
        (CacheDir, Uz) => "Kesh",
        (CacheDir, Ru) => "Кэш",
        (CacheDir, En) => "Cache",
        (MiniApps, Uz) => "Mini-ilovalar — birini tanlang, shu oynada ochiladi.",
        (MiniApps, Ru) => "Мини-приложения — выберите, и оно откроется в этом окне.",
        (MiniApps, En) => "Mini-apps — click one to load it into this window.",
        (NoMiniApps, Uz) => "Roʻyxatda mini-ilova yoʻq.",
        (NoMiniApps, Ru) => "В списке нет мини-приложений.",
        (NoMiniApps, En) => "No mini-apps listed.",
        (WebPage, Uz) => "Veb-sahifa",
        (WebPage, Ru) => "Веб-страница",
        (WebPage, En) => "Web page",
        (WebPageRouted, Uz) => {
            "Bu manzil WASM komponent emas, veb-sahifa, shuning uchun mini-ilovalar yoʻlidan chetga yuborildi."
        }
        (WebPageRouted, Ru) => {
            "Этот адрес — веб-страница, а не WASM-компонент, поэтому он не открывается как мини-приложение."
        }
        (WebPageRouted, En) => {
            "This address is a web page, not a WASM component, so the shell routed it away from the mini-app path."
        }
        #[cfg(not(feature = "webview"))]
        (WebPageRebuild, Uz) => "Uni shu yerda koʻrish uchun `--features webview` bilan qayta yigʻing.",
        #[cfg(not(feature = "webview"))]
        (WebPageRebuild, Ru) => "Чтобы показать её здесь, соберите с `--features webview`.",
        #[cfg(not(feature = "webview"))]
        (WebPageRebuild, En) => "Rebuild with `--features webview` to render it here.",
        #[cfg(not(feature = "webview"))]
        (WebPageNotHere, Uz) => "Bu qurilmada veb-sahifalar hali koʻrsatilmaydi.",
        #[cfg(not(feature = "webview"))]
        (WebPageNotHere, Ru) => "На этом устройстве веб-страницы пока не показываются.",
        #[cfg(not(feature = "webview"))]
        (WebPageNotHere, En) => "Web pages cannot be shown on this device yet.",
        (Updates, Uz) => "Yangilanishlar",
        (Updates, Ru) => "Обновления",
        (Updates, En) => "Updates",
        (CheckForUpdates, Uz) => "Yangilanishni tekshirish",
        (CheckForUpdates, Ru) => "Проверить обновления",
        (CheckForUpdates, En) => "Check for updates",
        (Checking, Uz) => "Tekshirilmoqda…",
        (Checking, Ru) => "Проверка…",
        (Checking, En) => "Checking…",
        (Newest, Uz) => "Bu eng soʻnggi versiya.",
        (Newest, Ru) => "Это последняя версия.",
        (Newest, En) => "This is the newest version.",
        (NewerOut, Uz) => "nimadir {version} chiqdi: {page}",
        (NewerOut, Ru) => "Вышел nimadir {version}: {page}",
        (NewerOut, En) => "nimadir {version} is out: {page}",
    }
}
