//! The settings store.
//!
//! Settings live in one plain JSON file (`settings.json` in the app config
//! directory). **No secret is ever stored here**: database passwords, SSH
//! passphrases and LLM keys go to the OS keychain, which is why the file can be
//! exported as-is. The types below deliberately have no field that could hold
//! one.
//!
//! Loading is forgiving: a missing file, a missing section, an unknown key or a
//! value of the wrong type falls back to the default for that section instead of
//! failing, and every number is clamped to a sane range. A file that is not JSON
//! at all is moved aside (`settings.json.corrupt`) so nothing is silently lost.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// Bumped when the file layout changes in a way that needs a migration.
pub const SETTINGS_VERSION: u32 = 1;

/// PostgreSQL truncates `application_name` to 63 bytes (NAMEDATALEN - 1).
pub const CLIENT_ID_MAX_LEN: usize = 63;

// ---------------------------------------------------------------- enums ----

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum $name { $($(#[$vmeta])* $variant),+ }
    };
}

string_enum!(UpdateChecks {
    #[default]
    Auto,
    Notify,
    Off
});
string_enum!(UpdateChannel {
    #[default]
    Stable,
    Beta
});
string_enum!(Theme {
    #[default]
    System,
    Light,
    Dark
});
string_enum!(Density {
    #[default]
    Comfortable,
    Compact
});
string_enum!(AutocompleteMode {
    #[default]
    Typing,
    OnDemand
});
string_enum!(KeywordCase {
    #[default]
    Upper,
    Lower,
    Preserve
});
string_enum!(ConfirmDestructive {
    Always,
    #[default]
    ProdOnly,
    Never
});
string_enum!(TimeZone {
    #[default]
    Local,
    Utc
});
string_enum!(NumberFormat {
    #[default]
    Plain,
    Grouped
});
string_enum!(BinaryDisplay {
    #[default]
    Hex,
    Base64
});
string_enum!(KnownHostsPolicy {
    #[default]
    Ask,
    Strict,
    AcceptNew
});

// ------------------------------------------------------------- sections ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct General {
    /// Session identity: `application_name` on PostgreSQL, client name on ClickHouse.
    pub client_id: String,
    pub restore_tabs: bool,
    pub update_checks: UpdateChecks,
    pub update_channel: UpdateChannel,
    /// False until the first-launch wizard has been completed or skipped.
    pub onboarding_completed: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            client_id: default_client_id(),
            restore_tabs: true,
            update_checks: UpdateChecks::default(),
            update_channel: UpdateChannel::default(),
            onboarding_completed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub theme: Theme,
    pub density: Density,
    /// Empty means "the platform's default monospace font".
    pub editor_font_family: String,
    pub editor_font_size: u32,
    pub grid_font_family: String,
    pub grid_font_size: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            density: Density::default(),
            editor_font_family: String::new(),
            editor_font_size: 13,
            grid_font_family: String::new(),
            grid_font_size: 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Editor {
    pub tab_size: u32,
    pub indent_with_tabs: bool,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub auto_close_brackets: bool,
    pub autocomplete: AutocompleteMode,
    pub formatter_keyword_case: KeywordCase,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            tab_size: 2,
            indent_with_tabs: false,
            word_wrap: false,
            line_numbers: true,
            auto_close_brackets: true,
            autocomplete: AutocompleteMode::default(),
            formatter_keyword_case: KeywordCase::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Execution {
    /// Rows per fetch. 0 means "no limit". Applied at fetch time, never by
    /// rewriting the user's SQL.
    pub default_row_limit: u32,
    /// Seconds; 0 means "no timeout".
    pub query_timeout_secs: u32,
    pub auto_commit: bool,
    pub stop_script_on_error: bool,
    pub confirm_destructive: ConfirmDestructive,
    pub result_memory_cap_mb: u32,
}

impl Default for Execution {
    fn default() -> Self {
        Self {
            default_row_limit: 1000,
            query_timeout_secs: 0,
            auto_commit: true,
            stop_script_on_error: true,
            confirm_destructive: ConfirmDestructive::default(),
            result_memory_cap_mb: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Results {
    pub null_text: String,
    pub date_time_format: String,
    pub time_zone: TimeZone,
    pub number_format: NumberFormat,
    pub binary_display: BinaryDisplay,
    pub max_cell_chars: u32,
    pub json_auto_detect: bool,
}

impl Default for Results {
    fn default() -> Self {
        Self {
            null_text: "NULL".into(),
            date_time_format: "YYYY-MM-DD HH:mm:ss".into(),
            time_zone: TimeZone::default(),
            number_format: NumberFormat::default(),
            binary_display: BinaryDisplay::default(),
            max_cell_chars: 500,
            json_auto_detect: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Connections {
    pub connect_timeout_secs: u32,
    /// Seconds; 0 disables keepalive.
    pub keepalive_secs: u32,
    pub auto_reconnect: bool,
    pub ssh_known_hosts: KnownHostsPolicy,
}

impl Default for Connections {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 10,
            keepalive_secs: 30,
            auto_reconnect: true,
            ssh_known_hosts: KnownHostsPolicy::default(),
        }
    }
}

/// One AI provider. Holds no secret: the key is in the OS keychain.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiProvider {
    pub id: String,
    pub kind: crate::ai::ProviderKind,
    pub name: String,
    /// Empty = the provider's standard endpoint.
    pub base_url: String,
    /// Azure OpenAI deployment name.
    pub deployment: String,
    /// Model for Ask and Explain.
    pub model: String,
    /// Model for fixes; empty = same as `model`.
    pub fix_model: String,
}

/// Non-secret AI preferences. Provider keys live in the OS keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ai {
    /// Configured providers. Their API keys are in the keychain, keyed by `id`.
    pub providers: Vec<AiProvider>,
    /// `id` of the provider used unless a request names another; empty = AI off.
    pub default_provider: String,
    pub auto_suggest_fixes: bool,
    /// Default for new connections; sample values are opt-in per connection.
    pub send_sample_values: bool,
    pub schema_token_budget: u32,
    pub request_timeout_secs: u32,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            default_provider: String::new(),
            auto_suggest_fixes: true,
            send_sample_values: false,
            schema_token_budget: 4000,
            request_timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Keyboard {
    /// Action id → key chord (e.g. `"settings.open": "Mod+,"`). Only rebound
    /// shortcuts are stored; everything else uses the built-in default.
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Privacy {
    pub keep_history: bool,
    pub history_retention_days: u32,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            keep_history: true,
            history_retention_days: 90,
        }
    }
}

// ------------------------------------------------------------- settings ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub appearance: Appearance,
    pub editor: Editor,
    pub execution: Execution,
    pub results: Results,
    pub connections: Connections,
    pub ai: Ai,
    pub keyboard: Keyboard,
    pub privacy: Privacy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: General::default(),
            appearance: Appearance::default(),
            editor: Editor::default(),
            execution: Execution::default(),
            results: Results::default(),
            connections: Connections::default(),
            ai: Ai::default(),
            keyboard: Keyboard::default(),
            privacy: Privacy::default(),
        }
    }
}

impl Settings {
    /// Builds settings from arbitrary JSON, keeping every section that parses
    /// and defaulting the ones that do not. Returns an error only when the
    /// input is not a JSON object at all.
    pub fn from_json_lenient(text: &str) -> Result<Self> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| Error::InvalidSettings(e.to_string()))?;
        let Value::Object(map) = value else {
            return Err(Error::InvalidSettings("expected a JSON object".into()));
        };

        fn section<T: Default + serde::de::DeserializeOwned>(
            map: &serde_json::Map<String, Value>,
            key: &str,
        ) -> T {
            map.get(key)
                .and_then(|v| lenient_section(v.clone()))
                .unwrap_or_default()
        }

        let mut settings = Settings {
            version: SETTINGS_VERSION,
            general: section(&map, "general"),
            appearance: section(&map, "appearance"),
            editor: section(&map, "editor"),
            execution: section(&map, "execution"),
            results: section(&map, "results"),
            connections: section(&map, "connections"),
            ai: section(&map, "ai"),
            keyboard: section(&map, "keyboard"),
            privacy: section(&map, "privacy"),
        };
        settings.normalize();
        Ok(settings)
    }

    /// Clamps every value into its valid range. Idempotent.
    pub fn normalize(&mut self) {
        self.version = SETTINGS_VERSION;

        self.general.client_id = sanitize_client_id(&self.general.client_id);

        let a = &mut self.appearance;
        a.editor_font_size = a.editor_font_size.clamp(8, 32);
        a.grid_font_size = a.grid_font_size.clamp(8, 32);
        a.editor_font_family = clean_text(&a.editor_font_family, 200);
        a.grid_font_family = clean_text(&a.grid_font_family, 200);

        self.editor.tab_size = self.editor.tab_size.clamp(1, 8);

        let x = &mut self.execution;
        x.default_row_limit = x.default_row_limit.min(1_000_000);
        x.query_timeout_secs = x.query_timeout_secs.min(86_400);
        x.result_memory_cap_mb = x.result_memory_cap_mb.clamp(16, 16_384);

        let r = &mut self.results;
        r.null_text = clean_text(&r.null_text, 32);
        r.date_time_format = clean_text(&r.date_time_format, 64);
        if r.date_time_format.is_empty() {
            r.date_time_format = Results::default().date_time_format;
        }
        r.max_cell_chars = r.max_cell_chars.clamp(20, 100_000);

        let c = &mut self.connections;
        c.connect_timeout_secs = c.connect_timeout_secs.clamp(1, 600);
        c.keepalive_secs = c.keepalive_secs.min(3_600);

        let ai = &mut self.ai;
        for provider in &mut ai.providers {
            for field in [
                &mut provider.id,
                &mut provider.name,
                &mut provider.base_url,
                &mut provider.deployment,
                &mut provider.model,
                &mut provider.fix_model,
            ] {
                *field = field.trim().to_string();
            }
            while provider.base_url.ends_with('/') {
                provider.base_url.pop();
            }
        }
        ai.providers.retain(|p| !p.id.is_empty());
        let mut seen = std::collections::HashSet::new();
        ai.providers.retain(|p| seen.insert(p.id.clone()));
        if !ai.providers.iter().any(|p| p.id == ai.default_provider) {
            ai.default_provider = ai
                .providers
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_default();
        }
        ai.schema_token_budget = ai.schema_token_budget.clamp(500, 200_000);
        ai.request_timeout_secs = ai.request_timeout_secs.clamp(5, 600);

        self.keyboard.overrides = std::mem::take(&mut self.keyboard.overrides)
            .into_iter()
            .filter(|(action, chord)| is_token(action, 64) && chord.len() <= 64)
            .take(256)
            .collect();

        let p = &mut self.privacy;
        p.history_retention_days = p.history_retention_days.clamp(1, 3_650);
    }

    pub fn to_pretty_json(&self) -> String {
        // Serializing plain structs of strings, numbers and booleans cannot fail.
        serde_json::to_string_pretty(self).expect("settings serialize")
    }
}

/// Deserializes one section, dropping individual fields that have the wrong
/// type so one bad value does not reset its whole section.
fn lenient_section<T: Default + serde::de::DeserializeOwned>(value: Value) -> Option<T> {
    if let Ok(parsed) = serde_json::from_value::<T>(value.clone()) {
        return Some(parsed);
    }
    let Value::Object(fields) = value else {
        return None;
    };
    let mut kept = serde_json::Map::new();
    for (key, field) in fields {
        let mut probe = serde_json::Map::new();
        probe.insert(key.clone(), field.clone());
        if serde_json::from_value::<T>(Value::Object(probe)).is_ok() {
            kept.insert(key, field);
        }
    }
    serde_json::from_value(Value::Object(kept)).ok()
}

fn clean_text(s: &str, max_chars: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_token(s: &str, max_len: usize) -> bool {
    !s.is_empty()
        && s.len() <= max_len
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

// ------------------------------------------------------------ client id ----

/// `rowboat-<os username>`, the spec's prefilled Client ID.
pub fn default_client_id() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    client_id_for_user(&user)
}

pub fn client_id_for_user(user: &str) -> String {
    let user = sanitize_client_id(user);
    if user.is_empty() {
        "rowboat".into()
    } else {
        sanitize_client_id(&format!("rowboat-{user}"))
    }
}

/// Makes a Client ID safe to send as PostgreSQL `application_name` and as the
/// ClickHouse client name: printable ASCII without spaces at the edges, at most
/// 63 bytes. An empty result falls back to the default.
pub fn sanitize_client_id(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .filter(|c| c.is_ascii_graphic())
        .take(CLIENT_ID_MAX_LEN)
        .collect();
    cleaned
}

// ---------------------------------------------------------------- store ----

/// Owns the settings file and the in-memory copy.
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Loads `path`, or starts from defaults when it does not exist. Never fails:
    /// an unreadable file is moved aside and replaced by defaults on next save.
    pub fn load(path: PathBuf) -> Self {
        let settings = match fs::read_to_string(&path) {
            Ok(text) => match Settings::from_json_lenient(&text) {
                Ok(settings) => settings,
                Err(_) => {
                    let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                    Self::defaults()
                }
            },
            Err(_) => Self::defaults(),
        };
        Self {
            path,
            current: Mutex::new(settings),
        }
    }

    fn defaults() -> Settings {
        let mut settings = Settings::default();
        settings.normalize();
        if settings.general.client_id.is_empty() {
            settings.general.client_id = "rowboat".into();
        }
        settings
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> Settings {
        self.lock().clone()
    }

    /// Validates, stores and persists new settings. Returns what was stored.
    pub fn set(&self, mut settings: Settings) -> Result<Settings> {
        settings.normalize();
        if settings.general.client_id.is_empty() {
            settings.general.client_id = default_client_id();
        }
        let mut current = self.lock();
        if *current != settings || !self.path.exists() {
            write_atomic(&self.path, settings.to_pretty_json().as_bytes())?;
            *current = settings.clone();
        }
        Ok(settings)
    }

    /// Resets to defaults, keeping the Client ID and onboarding state: those are
    /// identity, not preference.
    pub fn reset(&self) -> Result<Settings> {
        let current = self.get();
        let mut fresh = Settings::default();
        fresh.general.client_id = current.general.client_id;
        fresh.general.onboarding_completed = current.general.onboarding_completed;
        self.set(fresh)
    }

    pub fn export_to(&self, target: &Path) -> Result<()> {
        write_atomic(target, self.get().to_pretty_json().as_bytes())
    }

    pub fn import_from(&self, source: &Path) -> Result<Settings> {
        let text = fs::read_to_string(source)?;
        let settings = Settings::from_json_lenient(&text)?;
        self.set(settings)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Settings> {
        // A poisoned lock only means another thread panicked mid-read; the data
        // is a plain value and still usable.
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Writes via a temp file in the same directory and renames it into place, so
/// a crash mid-write never leaves a truncated settings file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

// ---------------------------------------------------------------- tests ----

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().join("settings.json"));
        (dir, store)
    }

    #[test]
    fn defaults_match_the_spec() {
        let s = Settings::default();
        assert_eq!(s.execution.default_row_limit, 1000);
        assert_eq!(s.execution.result_memory_cap_mb, 256);
        assert_eq!(s.appearance.theme, Theme::System);
        assert_eq!(
            s.execution.confirm_destructive,
            ConfirmDestructive::ProdOnly
        );
        assert!(s.ai.auto_suggest_fixes);
        assert!(!s.ai.send_sample_values, "sample values are opt-in");
        assert!(s.general.client_id.starts_with("rowboat"));
    }

    #[test]
    fn missing_file_gives_defaults_and_does_not_create_it() {
        let (_dir, store) = store();
        assert_eq!(store.get().execution.default_row_limit, 1000);
        assert!(!store.path().exists());
    }

    #[test]
    fn set_persists_and_reloads() {
        let (_dir, store) = store();
        let mut s = store.get();
        s.execution.default_row_limit = 5000;
        s.appearance.theme = Theme::Dark;
        s.general.client_id = "dom-laptop".into();
        store.set(s.clone()).unwrap();

        let reloaded = SettingsStore::load(store.path().to_path_buf());
        assert_eq!(reloaded.get(), s);
    }

    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let (dir, store) = store();
        store.set(store.get()).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let (_dir, store) = store();
        let mut s = store.get();
        s.appearance.editor_font_size = 500;
        s.editor.tab_size = 0;
        s.execution.result_memory_cap_mb = 1;
        s.connections.connect_timeout_secs = 0;
        let stored = store.set(s).unwrap();
        assert_eq!(stored.appearance.editor_font_size, 32);
        assert_eq!(stored.editor.tab_size, 1);
        assert_eq!(stored.execution.result_memory_cap_mb, 16);
        assert_eq!(stored.connections.connect_timeout_secs, 1);
    }

    #[test]
    fn unknown_keys_and_partial_files_are_tolerated() {
        let s = Settings::from_json_lenient(
            r#"{"execution": {"defaultRowLimit": 100, "futureKey": 1}, "futureSection": {}}"#,
        )
        .unwrap();
        assert_eq!(s.execution.default_row_limit, 100);
        assert!(s.execution.auto_commit, "unspecified fields keep defaults");
        assert_eq!(s.appearance, Appearance::default());
    }

    #[test]
    fn one_bad_field_does_not_reset_its_section() {
        let s = Settings::from_json_lenient(
            r#"{"appearance": {"theme": "neon", "editorFontSize": 16},
                "execution": {"defaultRowLimit": "lots", "autoCommit": false}}"#,
        )
        .unwrap();
        assert_eq!(s.appearance.theme, Theme::System);
        assert_eq!(s.appearance.editor_font_size, 16);
        assert_eq!(s.execution.default_row_limit, 1000);
        assert!(!s.execution.auto_commit);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        let store = SettingsStore::load(path.clone());
        assert_eq!(store.get().execution.default_row_limit, 1000);
        assert!(!path.exists());
        assert!(dir.path().join("settings.json.corrupt").exists());
    }

    #[test]
    fn client_id_is_sanitized_for_application_name() {
        assert_eq!(sanitize_client_id("  dom laptop \n"), "dom-laptop");
        assert_eq!(sanitize_client_id("jos\u{e9}\u{0}x"), "josx");
        assert_eq!(
            sanitize_client_id(&"a".repeat(100)).len(),
            CLIENT_ID_MAX_LEN
        );
        assert_eq!(client_id_for_user("dominic"), "rowboat-dominic");
        assert_eq!(client_id_for_user(""), "rowboat");
    }

    #[test]
    fn empty_client_id_falls_back_to_default() {
        let (_dir, store) = store();
        let mut s = store.get();
        s.general.client_id = "   ".into();
        let stored = store.set(s).unwrap();
        assert!(stored.general.client_id.starts_with("rowboat"));
    }

    #[test]
    fn reset_keeps_identity() {
        let (_dir, store) = store();
        let mut s = store.get();
        s.general.client_id = "dba-7".into();
        s.general.onboarding_completed = true;
        s.execution.default_row_limit = 42;
        store.set(s).unwrap();

        let reset = store.reset().unwrap();
        assert_eq!(reset.general.client_id, "dba-7");
        assert!(reset.general.onboarding_completed);
        assert_eq!(reset.execution.default_row_limit, 1000);
    }

    #[test]
    fn export_then_import_round_trips() {
        let (dir, store) = store();
        let mut s = store.get();
        s.results.null_text = "∅".into();
        s.keyboard
            .overrides
            .insert("settings.open".into(), "Mod+Shift+P".into());
        store.set(s.clone()).unwrap();

        let exported = dir.path().join("export.json");
        store.export_to(&exported).unwrap();

        let (_dir2, other) = self::store();
        assert_eq!(other.import_from(&exported).unwrap(), s);
    }

    #[test]
    fn import_rejects_non_json() {
        let (dir, store) = store();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "[1, 2, 3]").unwrap();
        assert!(store.import_from(&bad).is_err());
    }

    /// The UI mirrors this shape by hand (`src/lib/types.ts`). Both sides are
    /// pinned to one committed file: this test fails when the Rust defaults or
    /// field names drift from it, and `settingsContract.test.ts` fails when the
    /// TypeScript side does. Regenerate after an intended change with
    /// `UPDATE_FIXTURES=1 cargo test default_settings_match`.
    #[test]
    fn default_settings_match_the_shared_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/lib/settings.defaults.json"
        );
        let mut defaults = Settings::default();
        // The real default depends on who is logged in.
        defaults.general.client_id = "rowboat-dev".into();
        let expected = format!("{}\n", defaults.to_pretty_json());

        if std::env::var_os("UPDATE_FIXTURES").is_some() {
            fs::write(path, &expected).unwrap();
        }
        let actual = fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        assert_eq!(
            actual, expected,
            "settings.rs and src/lib/settings.defaults.json disagree"
        );
    }

    /// Every value the UI can send for an enum setting must be one the core
    /// accepts; otherwise the choice would be silently dropped on save.
    #[test]
    fn enum_values_offered_by_the_ui_are_accepted() {
        let cases = [
            ("general", "updateChecks", vec!["auto", "notify", "off"]),
            ("general", "updateChannel", vec!["stable", "beta"]),
            ("appearance", "theme", vec!["system", "light", "dark"]),
            ("appearance", "density", vec!["comfortable", "compact"]),
            ("editor", "autocomplete", vec!["typing", "on-demand"]),
            (
                "editor",
                "formatterKeywordCase",
                vec!["upper", "lower", "preserve"],
            ),
            (
                "execution",
                "confirmDestructive",
                vec!["always", "prod-only", "never"],
            ),
            ("results", "timeZone", vec!["local", "utc"]),
            ("results", "numberFormat", vec!["plain", "grouped"]),
            ("results", "binaryDisplay", vec!["hex", "base64"]),
            (
                "connections",
                "sshKnownHosts",
                vec!["ask", "strict", "accept-new"],
            ),
        ];
        for (section, key, values) in cases {
            for value in values {
                let json = format!(r#"{{"{section}": {{"{key}": "{value}"}}}}"#);
                let parsed = Settings::from_json_lenient(&json).unwrap();
                let round_trip: Value = serde_json::to_value(&parsed).unwrap();
                assert_eq!(
                    round_trip[section][key], value,
                    "{section}.{key} = {value} was not kept"
                );
            }
        }
    }

    /// The settings file is exportable precisely because it has no secrets.
    #[test]
    fn settings_file_has_no_secret_shaped_keys() {
        let json = Settings::default().to_pretty_json().to_lowercase();
        for needle in ["apikey", "api_key", "password", "passphrase", "secret"] {
            assert!(
                !json.contains(needle),
                "settings must not contain `{needle}`"
            );
        }
    }

    #[test]
    fn keyboard_overrides_are_validated() {
        let mut s = Settings::default();
        s.keyboard
            .overrides
            .insert("bad action!".into(), "Mod+K".into());
        s.keyboard
            .overrides
            .insert("editor.run".into(), "Mod+Enter".into());
        s.normalize();
        assert_eq!(s.keyboard.overrides.len(), 1);
        assert!(s.keyboard.overrides.contains_key("editor.run"));
    }
}
