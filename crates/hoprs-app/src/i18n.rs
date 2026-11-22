//! The app's text catalogs (Fluent) and the process-wide active language.
//!
//! `en.ftl` is the reference; `pt-BR.ftl` and `tr.ftl` translate it. A lookup tries the active
//! language, then English, then returns the key itself, and never panics: a missing key
//! or a formatting error is logged once per key.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

use crate::settings::LanguageChoice;

const EN_SRC: &str = include_str!("../i18n/en.ftl");
const PT_BR_SRC: &str = include_str!("../i18n/pt-BR.ftl");
const TR_SRC: &str = include_str!("../i18n/tr.ftl");

/// A language the interface is available in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    PtBr,
    Tr,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::En, Language::PtBr, Language::Tr];

    /// The BCP 47 tag: also the settings value and the catalog's file name.
    pub fn tag(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::PtBr => "pt-BR",
            Language::Tr => "tr",
        }
    }

    fn source(self) -> Option<&'static str> {
        match self {
            Language::En => None,
            Language::PtBr => Some(PT_BR_SRC),
            Language::Tr => Some(TR_SRC),
        }
    }
}

/// A value passed to a message.
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Str(&'a str),
    Num(i64),
    Float(f64),
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Str(s)
    }
}
impl<'a> From<&'a String> for Arg<'a> {
    fn from(s: &'a String) -> Self {
        Arg::Str(s)
    }
}
impl From<usize> for Arg<'_> {
    fn from(n: usize) -> Self {
        Arg::Num(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl From<u64> for Arg<'_> {
    fn from(n: u64) -> Self {
        Arg::Num(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl From<u32> for Arg<'_> {
    fn from(n: u32) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<i64> for Arg<'_> {
    fn from(n: i64) -> Self {
        Arg::Num(n)
    }
}
impl From<u16> for Arg<'_> {
    fn from(n: u16) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<i32> for Arg<'_> {
    fn from(n: i32) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<f64> for Arg<'_> {
    fn from(n: f64) -> Self {
        Arg::Float(n)
    }
}

/// `1000` as `1,000` (English) or `1.000` (Brazilian Portuguese, Turkish).
pub fn group_digits(lang: Language, n: u64) -> String {
    let sep = match lang {
        Language::En => ',',
        Language::PtBr | Language::Tr => '.',
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

fn format_number(lang: Language, value: f64) -> String {
    let negative = value.is_sign_negative() && value != 0.0;
    let abs = value.abs();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if abs.fract() == 0.0 && abs < u64::MAX as f64 {
        out.push_str(&group_digits(lang, abs as u64));
    } else {
        let text = abs.to_string();
        let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
        let int: u64 = int.parse().unwrap_or(0);
        out.push_str(&group_digits(lang, int));
        if !frac.is_empty() {
            out.push(match lang {
                Language::En => '.',
                Language::PtBr | Language::Tr => ',',
            });
            out.push_str(frac);
        }
    }
    out
}

// `set_formatter` takes a plain function pointer, so one function per language.
fn number_formatter<M>(lang: Language, value: &FluentValue, _memoizer: &M) -> Option<String> {
    match value {
        FluentValue::Number(n) => Some(format_number(lang, n.value)),
        _ => None,
    }
}
fn en_formatter<M>(value: &FluentValue, memoizer: &M) -> Option<String> {
    number_formatter(Language::En, value, memoizer)
}
fn pt_br_formatter<M>(value: &FluentValue, memoizer: &M) -> Option<String> {
    number_formatter(Language::PtBr, value, memoizer)
}
fn tr_formatter<M>(value: &FluentValue, memoizer: &M) -> Option<String> {
    number_formatter(Language::Tr, value, memoizer)
}

type Bundle = FluentBundle<FluentResource>;

fn bundle(lang: Language, source: &str) -> Bundle {
    let id: LanguageIdentifier = lang.tag().parse().expect("language tags are valid");
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    bundle.set_use_isolating(false);
    match lang {
        Language::En => bundle.set_formatter(Some(en_formatter)),
        Language::PtBr => bundle.set_formatter(Some(pt_br_formatter)),
        Language::Tr => bundle.set_formatter(Some(tr_formatter)),
    }
    match FluentResource::try_new(source.to_owned()) {
        Ok(resource) => {
            if let Err(errors) = bundle.add_resource(resource) {
                warn_once(&format!("{}:add", lang.tag()), || {
                    format!("catalog {}: {errors:?}", lang.tag())
                });
            }
        }
        Err((resource, errors)) => {
            warn_once(&format!("{}:parse", lang.tag()), || {
                format!("catalog {} has syntax errors: {errors:?}", lang.tag())
            });
            // Keep every entry that did parse.
            let _ = bundle.add_resource(resource);
        }
    }
    bundle
}

fn warn_once(key: &str, message: impl FnOnce() -> String) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let first = seen
        .lock()
        .map(|mut seen| seen.insert(key.to_owned()))
        .unwrap_or(false);
    if first {
        tracing::warn!("{}", message());
    }
}

/// A key that no catalog has, as a `'static` string (leaked once per distinct key).
fn leak_key(key: &str) -> &'static str {
    static KEYS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let keys = KEYS.get_or_init(Default::default);
    let Ok(mut keys) = keys.lock() else {
        return Box::leak(key.to_owned().into_boxed_str());
    };
    keys.entry(key.to_owned())
        .or_insert_with(|| Box::leak(key.to_owned().into_boxed_str()))
}

/// Message ids that take no variables, from a source's syntax tree.
fn message_ids(source: &str) -> Vec<(String, bool)> {
    ast::Resource::<&str>::default_from(source)
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(message) => {
                let mut vars = HashSet::new();
                if let Some(pattern) = &message.value {
                    pattern_variables(pattern, &mut vars);
                }
                Some((message.id.name.to_owned(), vars.is_empty()))
            }
            _ => None,
        })
        .collect()
}

trait ParseLenient<'a> {
    fn default_from(source: &'a str) -> Self;
}
impl<'a> ParseLenient<'a> for ast::Resource<&'a str> {
    fn default_from(source: &'a str) -> Self {
        match fluent_syntax::parser::parse(source) {
            Ok(resource) | Err((resource, _)) => resource,
        }
    }
}

fn pattern_variables<'a>(pattern: &ast::Pattern<&'a str>, out: &mut HashSet<&'a str>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            expression_variables(expression, out);
        }
    }
}

fn expression_variables<'a>(expression: &ast::Expression<&'a str>, out: &mut HashSet<&'a str>) {
    match expression {
        ast::Expression::Inline(inline) => inline_variables(inline, out),
        ast::Expression::Select { selector, variants } => {
            inline_variables(selector, out);
            for variant in variants {
                pattern_variables(&variant.value, out);
            }
        }
    }
}

fn inline_variables<'a>(inline: &ast::InlineExpression<&'a str>, out: &mut HashSet<&'a str>) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            out.insert(id.name);
        }
        ast::InlineExpression::Placeable { expression } => expression_variables(expression, out),
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            call_variables(arguments, out)
        }
        ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => call_variables(arguments, out),
        _ => {}
    }
}

fn call_variables<'a>(arguments: &ast::CallArguments<&'a str>, out: &mut HashSet<&'a str>) {
    for positional in &arguments.positional {
        inline_variables(positional, out);
    }
    for named in &arguments.named {
        inline_variables(&named.value, out);
    }
}

/// The messages of one language, with English behind it.
pub struct Catalog {
    lang: Language,
    primary: Option<Bundle>,
    english: Bundle,
    /// Messages without variables, formatted once (already through the fallback).
    plain: HashMap<&'static str, &'static str>,
}

impl Catalog {
    pub fn new(lang: Language) -> Catalog {
        Catalog::from_sources(lang, EN_SRC, lang.source())
    }

    /// A catalog from explicit sources (the tests build partial ones).
    pub(crate) fn from_sources(lang: Language, english: &str, primary: Option<&str>) -> Catalog {
        let mut catalog = Catalog {
            lang,
            primary: primary.map(|source| bundle(lang, source)),
            english: bundle(Language::En, english),
            plain: HashMap::new(),
        };
        let mut ids = message_ids(english);
        if let Some(source) = primary {
            ids.extend(message_ids(source));
        }
        for (id, plain) in ids {
            if !plain || catalog.plain.contains_key(id.as_str()) {
                continue;
            }
            if let Some(text) = catalog.format_checked(&id, &[]) {
                catalog.plain.insert(
                    Box::leak(id.into_boxed_str()),
                    Box::leak(text.into_boxed_str()),
                );
            }
        }
        catalog
    }

    /// A message without variables. A key that no catalog has comes back as itself.
    pub fn tr(&self, key: &str) -> &'static str {
        if let Some(text) = self.plain.get(key) {
            return text;
        }
        warn_once(&format!("missing:{key}"), || {
            format!("no text for the key `{key}` (or it needs arguments)")
        });
        leak_key(key)
    }

    pub fn tr_args(&self, key: &str, args: &[(&str, Arg)]) -> String {
        self.format_checked(key, args).unwrap_or_else(|| {
            warn_once(&format!("failed:{key}"), || {
                format!("cannot format the key `{key}`")
            });
            key.to_owned()
        })
    }

    /// Like `tr_args`, but `None` (and no warning) when neither language has the key or
    /// formatting reports any error, so the caller can fall back to its own text.
    pub fn tr_args_checked(&self, key: &str, args: &[(&str, Arg)]) -> Option<String> {
        self.format_checked(key, args)
    }

    /// The message in the catalog's language, else in English; `None` when neither has
    /// the key or formatting reports any error (a missing variable, say).
    fn format_checked(&self, key: &str, args: &[(&str, Arg)]) -> Option<String> {
        let mut fluent_args = FluentArgs::new();
        for (name, arg) in args {
            match *arg {
                Arg::Str(s) => fluent_args.set(*name, s),
                Arg::Num(n) => fluent_args.set(*name, n),
                Arg::Float(f) => fluent_args.set(*name, f),
            }
        }
        for bundle in self.primary.iter().chain(std::iter::once(&self.english)) {
            let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
            if errors.is_empty() {
                return Some(text.into_owned());
            }
            warn_once(&format!("error:{}:{key}", self.lang.tag()), || {
                format!("formatting `{key}`: {errors:?}")
            });
        }
        None
    }
}

pub(crate) fn catalog_for(lang: Language) -> &'static Catalog {
    static EN: OnceLock<Catalog> = OnceLock::new();
    static PT_BR: OnceLock<Catalog> = OnceLock::new();
    static TR: OnceLock<Catalog> = OnceLock::new();
    match lang {
        Language::En => EN.get_or_init(|| Catalog::new(Language::En)),
        Language::PtBr => PT_BR.get_or_init(|| Catalog::new(Language::PtBr)),
        Language::Tr => TR.get_or_init(|| Catalog::new(Language::Tr)),
    }
}

// The active language. Tests run in parallel threads, so under `cfg(test)` each thread
// has its own (starting at English): a test that switches language cannot disturb the
// ones asserting English.
#[cfg(not(test))]
mod active {
    use super::Language;
    use std::sync::atomic::{AtomicU8, Ordering};

    static ACTIVE: AtomicU8 = AtomicU8::new(0);

    pub fn set(lang: Language) {
        ACTIVE.store(lang as u8, Ordering::Relaxed);
    }
    pub fn get() -> Language {
        match ACTIVE.load(Ordering::Relaxed) {
            1 => Language::PtBr,
            2 => Language::Tr,
            _ => Language::En,
        }
    }
}

#[cfg(test)]
mod active {
    use super::Language;
    use std::cell::Cell;

    thread_local! {
        static ACTIVE: Cell<Language> = const { Cell::new(Language::En) };
    }

    pub fn set(lang: Language) {
        ACTIVE.with(|a| a.set(lang));
    }
    pub fn get() -> Language {
        ACTIVE.with(Cell::get)
    }
}

/// The language an OS locale (`pt_BR.UTF-8`, `tr-TR`, `C`, …) asks for: Brazilian
/// Portuguese when the language subtag is `pt`, Turkish when it is `tr` (any case, any
/// region or suffix), English otherwise, also for no locale, an empty one, `C` and
/// `POSIX`.
pub fn language_from_locale(locale: Option<&str>) -> Language {
    let tag = locale.unwrap_or("");
    let subtag = tag.split(['_', '-', '.', '@']).next().unwrap_or("");
    if subtag.eq_ignore_ascii_case("pt") {
        Language::PtBr
    } else if subtag.eq_ignore_ascii_case("tr") {
        Language::Tr
    } else {
        Language::En
    }
}

/// The language a setting means; `System` asks the OS.
pub fn resolve(choice: LanguageChoice) -> Language {
    match choice {
        LanguageChoice::Fixed(lang) => lang,
        LanguageChoice::System => language_from_locale(sys_locale::get_locale().as_deref()),
    }
}

pub fn set_language(lang: Language) {
    active::set(lang);
}

pub fn language() -> Language {
    active::get()
}

/// A message without variables, in the active language.
pub fn tr(key: &str) -> &'static str {
    catalog_for(language()).tr(key)
}

/// A message with variables, in the active language.
pub fn tr_args(key: &str, args: &[(&str, Arg)]) -> String {
    catalog_for(language()).tr_args(key, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_args_checked_is_none_on_missing_key_or_variable() {
        let en = "hello = Hello { $name }\nplain = Plain\n";
        let pt = "hello = Ola { $name }\n";
        let c = Catalog::from_sources(Language::PtBr, en, Some(pt));
        assert_eq!(
            c.tr_args_checked("hello", &[("name", "Ana".into())])
                .as_deref(),
            Some("Ola Ana")
        );
        assert_eq!(c.tr_args_checked("plain", &[]).as_deref(), Some("Plain"));
        assert_eq!(c.tr_args_checked("hello", &[]), None);
        assert_eq!(c.tr_args_checked("nope", &[]), None);
        assert_eq!(
            catalog_for(Language::En).tr_args_checked("no-such-key", &[]),
            None
        );
    }

    const SAME_IN_BOTH: &[&str] = &[
        "app-title",
        "language-english",
        "language-pt-br",
        "language-tr",
        // Product names, symbols and shortcut labels.
        "brand",
        "compare-ellipsis",
        "export-destination-drive",
        "go-to-anything-key",
        "key-settings",
        "key-settings-macos",
        "palette-key-copy",
        "palette-key-paste",
        "palette-key-find",
        "ssh-options",
        "ftp-options",
        // Terms kept in English, or words that are the same in Portuguese.
        "palette-group-site",
        "palette-group-workspace",
        "palette-kind-terminal",
        "workspaces",
        "field-host",
        "workspace-sites",
        "open-as-terminal",
        "auth-keyboard-interactive",
        // File panes, log levels and file actions: symbols, units and loanwords.
        "local",
        "log",
        "log-info",
        "log-debug",
        "upload",
        "download",
        "perm-octal",
        "sort-ascending",
        "sort-descending",
        "parent-dir",
        "path-separator",
        "home-dir",
        "no-size",
        "key-new-folder",
        "key-rename",
        "key-delete",
        "key-edit",
        "key-refresh",
        "key-transfer",
        "size-unit-b",
        "size-unit-kb",
        "size-unit-mb",
        "size-unit-gb",
        "size-unit-tb",
        "size-unit-pb",
        "size-unit-eb",
        // Settings and backup: product terms, units and symbols.
        "label-host",
        "field-passphrase",
        "settings-keepalive",
        "settings-terminal",
        "decrease",
        "increase",
        "backup",
        "editor-command",
    ];

    /// Words that read the same in Turkish as in English.
    const SAME_IN_TR: &[&str] = &["field-port", "month-3", "month-5"];

    fn parse(source: &str) -> ast::Resource<&str> {
        ast::Resource::default_from(source)
    }

    fn variables(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
        let mut map = std::collections::BTreeMap::new();
        for entry in &parse(source).body {
            if let ast::Entry::Message(message) = entry {
                let mut vars = HashSet::new();
                if let Some(pattern) = &message.value {
                    pattern_variables(pattern, &mut vars);
                }
                let mut vars: Vec<String> = vars.into_iter().map(str::to_owned).collect();
                vars.sort();
                map.insert(message.id.name.to_owned(), vars);
            }
        }
        map
    }

    #[test]
    fn catalogs_parse_without_errors() {
        for (name, source) in [("en", EN_SRC), ("pt-BR", PT_BR_SRC), ("tr", TR_SRC)] {
            let resource = match FluentResource::try_new(source.to_owned()) {
                Ok(resource) => resource,
                Err((_, errors)) => panic!("{name}.ftl: {errors:?}"),
            };
            // Adding to a bundle fails on a duplicate message id.
            let mut bundle =
                FluentBundle::new_concurrent(vec![Language::En.tag().parse().unwrap()]);
            if let Err(errors) = bundle.add_resource(resource) {
                panic!("{name}.ftl: {errors:?}");
            }
        }
    }

    #[test]
    fn same_keys_and_variables_in_both_languages() {
        assert_eq!(variables(EN_SRC), variables(PT_BR_SRC));
        assert_eq!(variables(EN_SRC), variables(TR_SRC));
        assert!(variables(EN_SRC).contains_key("site-count"));
        assert_eq!(variables(EN_SRC)["site-count"], ["count"]);
    }

    #[test]
    fn pt_br_values_differ_from_english() {
        let en = Catalog::new(Language::En);
        let pt = Catalog::new(Language::PtBr);
        for (id, plain) in message_ids(EN_SRC) {
            if plain && !SAME_IN_BOTH.contains(&id.as_str()) {
                assert_ne!(en.tr(&id), pt.tr(&id), "{id} is not translated");
            }
        }
    }

    #[test]
    fn tr_values_differ_from_english() {
        let en = Catalog::new(Language::En);
        let tr = Catalog::new(Language::Tr);
        for (id, plain) in message_ids(EN_SRC) {
            let same = SAME_IN_BOTH.contains(&id.as_str()) || SAME_IN_TR.contains(&id.as_str());
            if plain && !same {
                assert_ne!(en.tr(&id), tr.tr(&id), "{id} is not translated to Turkish");
            }
        }
    }

    #[test]
    fn missing_key_falls_back_to_english_then_key() {
        let partial = Catalog::from_sources(Language::PtBr, EN_SRC, Some("app-title = x\n"));
        assert_eq!(partial.tr("app-title"), "x");
        assert_eq!(partial.tr("already-running"), "hoprs is already running");
        assert_eq!(
            partial.tr_args("site-count", &[("count", 3.into())]),
            "3 sites"
        );
        assert_eq!(partial.tr("no-such-key"), "no-such-key");
        assert_eq!(partial.tr_args("no-such-key", &[]), "no-such-key");
    }

    #[test]
    fn missing_argument_does_not_leak_a_placeholder() {
        let text = Catalog::new(Language::En).tr_args("site-count", &[]);
        assert!(!text.contains('$') && !text.contains('{'), "{text}");
    }

    #[test]
    fn no_isolation_marks() {
        for lang in Language::ALL {
            let text = Catalog::new(lang).tr_args("site-count", &[("count", 3.into())]);
            assert!(!text.contains('\u{2068}') && !text.contains('\u{2069}'));
        }
    }

    #[test]
    fn numbers_group_per_language() {
        let args = [("count", Arg::from(1000usize))];
        assert!(
            Catalog::new(Language::PtBr)
                .tr_args("site-count", &args)
                .contains("1.000")
        );
        assert!(
            Catalog::new(Language::En)
                .tr_args("site-count", &args)
                .contains("1,000")
        );
        assert!(
            Catalog::new(Language::Tr)
                .tr_args("site-count", &args)
                .contains("1.000")
        );
        assert_eq!(format_number(Language::Tr, 4.1), "4,1");
        assert_eq!(group_digits(Language::En, 999), "999");
        assert_eq!(group_digits(Language::En, 1_234_567), "1,234,567");
        assert_eq!(format_number(Language::PtBr, 4.1), "4,1");
        assert_eq!(format_number(Language::En, 1234.5), "1,234.5");
    }

    #[test]
    fn site_count_plurals() {
        let en = Catalog::new(Language::En);
        let pt = Catalog::new(Language::PtBr);
        let count = |c: &Catalog, n: usize| c.tr_args("site-count", &[("count", n.into())]);
        assert_eq!(count(&en, 0), "0 sites");
        assert_eq!(count(&en, 1), "1 site");
        assert_eq!(count(&en, 2), "2 sites");
        assert_eq!(count(&pt, 0), "0 sites");
        assert_eq!(count(&pt, 1), "1 site");
        assert_eq!(count(&pt, 2), "2 sites");
        let tr = Catalog::new(Language::Tr);
        assert_eq!(count(&tr, 0), "0 site");
        assert_eq!(count(&tr, 1), "1 site");
        assert_eq!(count(&tr, 1000), "1.000 site");
    }

    #[test]
    fn language_is_per_test_thread_and_switches() {
        assert_eq!(language(), Language::En);
        set_language(Language::PtBr);
        assert_eq!(tr("already-running"), "O hoprs já está em execução");
        set_language(Language::En);
        assert_eq!(tr("already-running"), "hoprs is already running");
        set_language(Language::Tr);
        assert_eq!(tr("already-running"), "hoprs zaten çalışıyor");
        set_language(Language::En);
    }

    #[test]
    fn strings_rs_keys_exist() {
        let source = include_str!("ui/strings.rs");
        let en = variables(EN_SRC);
        let mut found = 0;
        for call in ["tr", "tr_args"] {
            for (at, _) in source.match_indices(call) {
                let before = source[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                // `(`, then optional whitespace (rustfmt wraps long calls), then the quote.
                let rest = source[at + call.len()..].strip_prefix('(');
                let Some(rest) = rest.map(str::trim_start) else {
                    continue;
                };
                let Some(rest) = rest.strip_prefix('"') else {
                    continue;
                };
                let key = &rest[..rest.find('"').expect("closing quote")];
                assert!(
                    en.contains_key(key),
                    "strings.rs uses `{key}`, not in en.ftl"
                );
                found += 1;
            }
        }
        assert!(found >= 4, "found only {found} keys");
    }

    /// Every English message is looked up by a literal in `strings.rs`; only the coded
    /// errors (`error-*`) are looked up by a code computed at run time.
    #[test]
    fn every_en_key_is_used() {
        let source = include_str!("ui/strings.rs");
        let production = &source[..source.find("#[cfg(test)]").expect("test module")];
        let unused: Vec<String> = message_ids(EN_SRC)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| !id.starts_with("error-"))
            .filter(|id| !production.contains(&format!("\"{id}\"")))
            .collect();
        assert!(
            unused.is_empty(),
            "keys in en.ftl never looked up: {unused:?}"
        );
        // The error keys are looked up by code: each must be a code some call site writes.
        let codes: std::collections::BTreeSet<String> = coded_sites()
            .into_iter()
            .map(|site| format!("error-{}", site.code))
            .collect();
        let dead: Vec<String> = message_ids(EN_SRC)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| id.starts_with("error-") && !codes.contains(id))
            .collect();
        assert!(dead.is_empty(), "error keys no call site uses: {dead:?}");
    }

    #[test]
    fn locale_mapping() {
        for pt in [
            "pt_BR.UTF-8",
            "pt-BR",
            "pt_PT",
            "pt",
            "PT",
            "pt_BR.UTF-8@euro",
        ] {
            assert_eq!(language_from_locale(Some(pt)), Language::PtBr, "{pt}");
        }
        for other in [
            "en_US.UTF-8",
            "de_DE",
            "C",
            "C.UTF-8",
            "POSIX",
            "",
            "ptx",
            "en",
        ] {
            assert_eq!(language_from_locale(Some(other)), Language::En, "{other}");
        }
        for tr in ["tr_TR.UTF-8", "tr-TR", "tr", "TR", "tr_TR.UTF-8@euro"] {
            assert_eq!(language_from_locale(Some(tr)), Language::Tr, "{tr}");
        }
        assert_eq!(language_from_locale(Some("trx")), Language::En);
        assert_eq!(language_from_locale(None), Language::En);
    }

    #[test]
    fn resolve_fixed_ignores_the_os() {
        use crate::settings::LanguageChoice;
        assert_eq!(
            resolve(LanguageChoice::Fixed(Language::PtBr)),
            Language::PtBr
        );
        assert_eq!(resolve(LanguageChoice::Fixed(Language::Tr)), Language::Tr);
        assert_eq!(resolve(LanguageChoice::Fixed(Language::En)), Language::En);
    }

    // ---- error codes in the core crates ---------------------------------------------

    /// One `Error::coded(kind, "code", message)` call found in a crate's source.
    struct CodedSite {
        file: String,
        code: String,
        /// The message expression as written, e.g. `"Not connected"` or `format!(..)`.
        message: String,
        args: Vec<String>,
    }

    /// Index just past the `)` that closes the `(` at `open`, skipping string literals.
    fn closing_paren(src: &str, open: usize) -> usize {
        let b = src.as_bytes();
        let (mut depth, mut i) = (0usize, open);
        while i < b.len() {
            match b[i] {
                b'"' => {
                    i += 1;
                    while b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        panic!("unbalanced parenthesis");
    }

    /// Splits `inner` at its top-level commas.
    fn top_level_parts(inner: &str) -> Vec<String> {
        let b = inner.as_bytes();
        let (mut parts, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0, 0);
        while i < b.len() {
            match b[i] {
                b'"' => {
                    i += 1;
                    while b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(inner[start..i].trim().to_owned());
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        let last = inner[start..].trim();
        if !last.is_empty() {
            parts.push(last.to_owned());
        }
        parts
    }

    /// The content of a leading Rust string literal (`"…"`), with `\"` and `\\` resolved and
    /// a `\` line continuation swallowing the next line's indentation.
    fn string_literal(text: &str) -> Option<String> {
        let rest = text.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next()? {
                    c if c == '\n' || (c == '\r' && chars.peek() == Some(&'\n')) => {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// The named values of a `format!` template: `{name}` placeholders. `None` when the
    /// template has anything this scan cannot map (positional or `:?` placeholders).
    fn template_variables(template: &str) -> Option<Vec<String>> {
        let mut vars = Vec::new();
        let mut rest = template.replace("{{", "").replace("}}", "");
        while let Some(open) = rest.find('{') {
            let close = rest[open..].find('}')? + open;
            let name = &rest[open + 1..close];
            let is_const = name.chars().all(|c| c.is_ascii_uppercase() || c == '_');
            let is_ident = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !is_const && !is_ident {
                return None;
            }
            if is_ident && !vars.contains(&name.to_owned()) {
                vars.push(name.to_owned());
            }
            rest = rest[close + 1..].to_owned();
        }
        vars.sort();
        Some(vars)
    }

    /// Production source of `path`: everything before its `mod tests`.
    fn production_source(source: &str) -> &str {
        let mut from = 0;
        while let Some(at) = source[from..].find("#[cfg(test)]") {
            let at = from + at;
            let after = source[at + "#[cfg(test)]".len()..].trim_start();
            if after.starts_with("mod ") {
                return &source[..at];
            }
            from = at + 1;
        }
        source
    }

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    rust_files(&path, out);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && name != "testing.rs" {
                out.push(path);
            }
        }
    }

    fn coded_sites() -> Vec<CodedSite> {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&crates).unwrap() {
            let src = entry.unwrap().path().join("src");
            if src.is_dir() {
                rust_files(&src, &mut files);
            }
        }
        files.sort();
        let mut sites = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let src = production_source(&text);
            let mut from = 0;
            while let Some(at) = src[from..].find("Error::coded(") {
                let open = from + at + "Error::coded".len();
                let end = closing_paren(src, open);
                from = end;
                let parts = top_level_parts(&src[open + 1..end - 1]);
                assert_eq!(parts.len(), 3, "{}: {:?}", file.display(), parts);
                // A code computed at run time (a variable) is covered by its literal sites.
                let Some(code) = string_literal(&parts[1]) else {
                    continue;
                };
                let mut args = Vec::new();
                let mut tail = end;
                loop {
                    let rest = src[tail..].trim_start();
                    let skipped = src.len() - tail - rest.len();
                    let call = if rest.starts_with(".arg(") {
                        ".arg"
                    } else if rest.starts_with(".with_detail(") {
                        ".with_detail"
                    } else {
                        break;
                    };
                    let open = tail + skipped + call.len();
                    let close = closing_paren(src, open);
                    if call == ".arg" {
                        let name = string_literal(src[open + 1..close - 1].trim_start())
                            .expect("arg name literal");
                        args.push(name);
                    }
                    tail = close;
                }
                args.sort();
                sites.push(CodedSite {
                    file: file.display().to_string(),
                    code,
                    message: parts[2].clone(),
                    args,
                });
            }
        }
        sites
    }

    /// The single-line value of `key` in a catalog source.
    fn single_line(source: &str, key: &str) -> Option<String> {
        source
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(" = "))
            .map(str::to_owned)
    }

    #[test]
    fn error_codes_exist_in_both_catalogs() {
        let sites = coded_sites();
        assert!(
            sites.len() >= 100,
            "found only {} coded errors",
            sites.len()
        );
        let en = variables(EN_SRC);
        let pt = variables(PT_BR_SRC);
        let tr = variables(TR_SRC);
        let mut texts: std::collections::BTreeMap<String, (String, Vec<String>)> =
            std::collections::BTreeMap::new();
        for site in &sites {
            let key = format!("error-{}", site.code);
            let at = format!("{} ({})", site.code, site.file);
            let Some(en_vars) = en.get(&key) else {
                panic!("{key} is not in en.ftl: {at}");
            };
            assert!(pt.contains_key(&key), "{key} is not in pt-BR.ftl: {at}");
            assert_eq!(
                en_vars, &site.args,
                "{at}: arguments passed vs. variables of en.ftl"
            );
            assert_eq!(&pt[&key], en_vars, "{at}: pt-BR variables");
            assert!(tr.contains_key(&key), "{key} is not in tr.ftl: {at}");
            assert_eq!(&tr[&key], en_vars, "{at}: tr variables");
            // The English the call writes: a literal or a `format!` template.
            let template = if let Some(literal) = string_literal(&site.message) {
                Some(literal)
            } else {
                site.message
                    .strip_prefix("format!(")
                    .and_then(|rest| string_literal(rest.trim_start()))
            };
            if let Some(template) = &template
                && let Some(vars) = template_variables(template)
            {
                assert_eq!(
                    vars, site.args,
                    "{at}: variables of the message vs. arguments"
                );
                let mut shown = template.replace("{{", "\u{1}").replace("}}", "\u{2}");
                for var in &vars {
                    shown = shown.replace(&format!("{{{var}}}"), &format!("{{ ${var} }}"));
                }
                let shown = shown.replace('\u{1}', "{").replace('\u{2}', "}");
                if let Some(value) = single_line(EN_SRC, &key) {
                    assert_eq!(value, shown, "{at}: en.ftl text differs from the message");
                }
            }
            // One code is one English message with one set of arguments.
            let text = template.unwrap_or_else(|| site.message.clone());
            match texts.get(&site.code) {
                Some((first, args)) => {
                    assert_eq!(
                        first, &text,
                        "code {} has two different messages",
                        site.code
                    );
                    assert_eq!(args, &site.args, "code {} has two argument sets", site.code);
                }
                None => {
                    texts.insert(site.code.clone(), (text, site.args.clone()));
                }
            }
        }
    }

    #[test]
    fn string_literal_accepts_a_crlf_line_continuation() {
        assert_eq!(
            string_literal("\"ab\\\r\n      cd\" x").as_deref(),
            Some("abcd")
        );
        assert_eq!(string_literal("\"ab\\\n   cd\"").as_deref(), Some("abcd"));
    }
}
