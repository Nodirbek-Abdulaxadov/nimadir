//! `rates` — exchange rates from the Central Bank of Uzbekistan, with a
//! converter. The first app here not written to show off a capability: it is
//! the one a person in Tashkent would open.
//!
//! It uses most of what an app has: the network (`cbu.uz`, declared in its
//! manifest and allowed by the user), storage (the last rates, so it opens
//! with something offline — kept between visits once it is signed), layout,
//! a field, a choice, and the user's locale for its words and currency names.
//!
//! Values: field 0 is the amount, choice 1 the currency. Taps: 0 is Refresh.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::sync::Mutex;

use nimadir::shell::host_api::{
    fetch, fetch_poll, locale, storage_get, storage_set, ui_begin, ui_button, ui_choice, ui_end,
    ui_heading, ui_label, ui_text_edit, HttpRequest,
};
use serde::Deserialize;

const SOURCE: &str = "https://cbu.uz/uz/arkhiv-kursov-valyut/json/";
/// The currencies the converter offers; the list below shows them all.
const CONVERT: [&str; 3] = ["USD", "EUR", "RUB"];

/// One entry of the bank's JSON, as it is published.
#[derive(Deserialize, Clone)]
#[allow(non_snake_case)]
struct Rate {
    Ccy: String,
    CcyNm_UZ: String,
    CcyNm_RU: String,
    CcyNm_EN: String,
    Nominal: String,
    Rate: String,
    Diff: String,
    Date: String,
}

impl Rate {
    fn name(&self, lang: Lang) -> &str {
        match lang {
            Lang::Uz => &self.CcyNm_UZ,
            Lang::Ru => &self.CcyNm_RU,
            Lang::En => &self.CcyNm_EN,
        }
    }

    /// Soʻm for one unit — the bank quotes some currencies per 10 or 100.
    fn per_unit(&self) -> Option<f64> {
        let rate: f64 = self.Rate.parse().ok()?;
        let nominal: f64 = self.Nominal.parse().ok()?;
        (nominal > 0.0).then(|| rate / nominal)
    }
}

#[derive(Clone, Copy)]
enum Lang {
    Uz,
    Ru,
    En,
}

impl Lang {
    fn from_locale(tag: &str) -> Self {
        match tag.split('-').next().unwrap_or("") {
            "uz" => Lang::Uz,
            "ru" => Lang::Ru,
            _ => Lang::En,
        }
    }
}

/// The few words the app says, in the three languages its users read.
fn say(lang: Lang, what: Words) -> &'static str {
    use Words::*;
    match (lang, what) {
        (Lang::Uz, Title) => "Valyuta kurslari",
        (Lang::Ru, Title) => "Курсы валют",
        (Lang::En, Title) => "Exchange rates",
        (Lang::Uz, Bank) => "Oʻzbekiston Respublikasi Markaziy banki",
        (Lang::Ru, Bank) => "Центральный банк Республики Узбекистан",
        (Lang::En, Bank) => "Central Bank of Uzbekistan",
        (Lang::Uz, Converter) => "Konvertor",
        (Lang::Ru, Converter) => "Конвертер",
        (Lang::En, Converter) => "Converter",
        (Lang::Uz, All) => "Barcha valyutalar",
        (Lang::Ru, All) => "Все валюты",
        (Lang::En, All) => "All currencies",
        (Lang::Uz, Refresh) => "Yangilash",
        (Lang::Ru, Refresh) => "Обновить",
        (Lang::En, Refresh) => "Refresh",
        (Lang::Uz, Loading) => "Yuklanmoqda…",
        (Lang::Ru, Loading) => "Загрузка…",
        (Lang::En, Loading) => "Loading…",
        (Lang::Uz, Som) => "soʻm",
        (Lang::Ru, Som) => "сум",
        (Lang::En, Som) => "soʻm",
    }
}

#[derive(Clone, Copy)]
enum Words {
    Title,
    Bank,
    Converter,
    All,
    Refresh,
    Loading,
    Som,
}

struct State {
    rates: Vec<Rate>,
    pending: Option<u32>,
    problem: Option<String>,
    amount: String,
    currency: u32,
    asked: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    rates: Vec::new(),
    pending: None,
    problem: None,
    amount: String::new(),
    currency: 0,
    asked: false,
});

struct Rates;

impl Guest for Rates {
    /// Opens on the last rates it kept, if any, before asking for new ones.
    fn init() {
        let mut s = STATE.lock().unwrap();
        s.amount = "100".into();
        if let Some(kept) = storage_get("rates").and_then(|t| serde_json::from_str(&t).ok()) {
            s.rates = kept;
        }
    }

    fn update() {
        let lang = Lang::from_locale(&locale());
        let mut s = STATE.lock().unwrap();

        if !s.asked {
            s.asked = true;
            request(&mut s);
        }
        if let Some(id) = s.pending {
            if let Some(answer) = fetch_poll(id) {
                s.pending = None;
                receive(&mut s, answer);
            }
        }

        ui_heading(say(lang, Words::Title), 1);
        match (s.rates.first(), &s.problem, s.pending) {
            (_, _, Some(_)) if s.rates.is_empty() => ui_label(say(lang, Words::Loading)),
            (Some(r), _, _) => ui_label(&format!("{} — {}", say(lang, Words::Bank), r.Date)),
            (None, _, _) => {}
        }
        if let Some(problem) = &s.problem {
            ui_label(problem);
        }

        if !s.rates.is_empty() {
            ui_begin("section");
            ui_heading(say(lang, Words::Converter), 2);
            ui_begin("row");
            s.amount = ui_text_edit(&s.amount);
            let options: Vec<String> = CONVERT.iter().map(|c| c.to_string()).collect();
            s.currency = ui_choice(&options, s.currency);
            ui_end();
            let code = CONVERT[s.currency as usize];
            let amount: Option<f64> = s.amount.trim().replace(',', ".").parse().ok();
            let rate = s
                .rates
                .iter()
                .find(|r| r.Ccy == code)
                .and_then(Rate::per_unit);
            if let (Some(amount), Some(rate)) = (amount, rate) {
                ui_label(&format!(
                    "{} {code} = {} {}",
                    s.amount.trim(),
                    group(amount * rate),
                    say(lang, Words::Som)
                ));
            }
            ui_end();

            ui_heading(say(lang, Words::All), 3);
            ui_begin("scroll");
            for r in &s.rates {
                ui_begin("row");
                ui_label(&format!("{} {}", r.Ccy, r.name(lang)));
                ui_label(&format!("{} {}", r.Rate, signed(&r.Diff)));
                ui_end();
            }
            ui_end();
        }

        if s.pending.is_none() && ui_button(say(lang, Words::Refresh)) {
            request(&mut s);
        }
    }
}

fn request(s: &mut State) {
    let get = HttpRequest {
        method: "GET".into(),
        url: SOURCE.into(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    match fetch(&get) {
        Ok(id) => {
            s.pending = Some(id);
            s.problem = None;
        }
        Err(why) => s.problem = Some(why),
    }
}

fn receive(s: &mut State, answer: Result<nimadir::shell::host_api::HttpResponse, String>) {
    let response = match answer {
        Ok(r) if r.status == 200 => r,
        Ok(r) => return s.problem = Some(format!("cbu.uz answered {}", r.status)),
        Err(why) => return s.problem = Some(why),
    };
    match serde_json::from_slice::<Vec<Rate>>(&response.body) {
        Ok(rates) if !rates.is_empty() => {
            // Kept as published, so the next visit can open with it.
            if let Ok(text) = std::str::from_utf8(&response.body) {
                let _ = storage_set("rates", text);
            }
            s.rates = rates;
            s.problem = None;
        }
        _ => s.problem = Some("cbu.uz sent something that is not a rate list".into()),
    }
}

/// `1 204 472,00` — thousands apart, as the amounts are written here.
fn group(value: f64) -> String {
    let cents = (value * 100.0).round() as i64;
    let (whole, frac) = (cents / 100, (cents % 100).abs());
    let digits = whole.abs().to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(d);
    }
    format!("{}{out},{frac:02}", if whole < 0 { "-" } else { "" })
}

/// The bank's change, with its sign shown either way.
fn signed(diff: &str) -> String {
    if diff.starts_with('-') {
        diff.to_string()
    } else {
        format!("+{diff}")
    }
}

export!(Rates);
