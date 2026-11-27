//! The model of the Settings window, on every platform: the values of one context from the config file and the tray's
//! file, their edits, the checks, and the locked writes of Apply.

use cicada::runtime::WorkerConfig;
use cicada_app::{ContextFile, Env, check_server_url};

use crate::tray_file::{TrayFile, WorkerSettings, locate};

/// The most turns at once the form accepts.
pub(crate) const MAX_CONCURRENCY: usize = 1_000;
/// The longest name of a new context.
const MAX_NAME: usize = 64;

/// A field of the form that an error belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// The context: its name, or that there is none.
    Context,
    /// The server URL.
    Server,
    /// The type switches.
    Types,
    /// The turns at once.
    Concurrency,
}

/// Why Apply did not finish.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ApplyError {
    /// The values are not valid; nothing was written.
    Invalid(Vec<(Field, String)>),
    /// The config file could not be written; nothing was saved.
    Failed(String),
    /// The connection was saved, or needed no change, but the worker settings were not saved; no reconnect.
    Partial(String),
}

/// The values of one context the window edits. The token is never shown in `Debug`, so the type has none.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Values {
    /// The server URL.
    pub server: String,
    /// The token; empty for none.
    pub token: String,
    /// The selected registered types, in the order the app registers them.
    pub types: Vec<String>,
    /// The turns at once, as typed.
    pub concurrency: String,
}

#[derive(Clone, PartialEq, Eq)]
enum Selected {
    Existing(String),
    New(String),
}

/// The Settings form of one selected context.
pub(crate) struct SettingsForm {
    env: Env,
    registered: Vec<&'static str>,
    names: Vec<String>,
    current: Option<String>,
    selected: Option<Selected>,
    saved: Values,
    /// Whether `tray.toml` had an entry for the selected context; without one, Apply saves the shown worker values.
    has_entry: bool,
    /// The values the user edits.
    pub edited: Values,
}

impl SettingsForm {
    /// Reads both files of `env` and selects `context` when the config file has it, else the current context, else the
    /// first; none when the file has no context.
    pub fn load(env: &Env, registered: &[&'static str], context: Option<&str>) -> Result<Self, String> {
        let mut form = Self {
            env: env.clone(),
            registered: registered.to_vec(),
            names: Vec::new(),
            current: None,
            selected: None,
            saved: Values::empty(),
            has_entry: false,
            edited: Values::empty(),
        };
        form.read(context)?;
        Ok(form)
    }

    /// Reads the files again and selects `wanted` as [`Self::load`] does.
    fn read(&mut self, wanted: Option<&str>) -> Result<(), String> {
        let file = ContextFile::load(&self.env).map_err(|e| format!("{e:#}"))?;
        let tray = TrayFile::load(&locate(&file)?).map_err(|e| format!("{e:#}"))?;
        self.names = file.names().map(ToOwned::to_owned).collect();
        self.current = file.current().map(ToOwned::to_owned);
        let has = |name: &&str| self.names.iter().any(|n| n == name);
        let name =
            wanted.filter(has).or(self.current.as_deref().filter(has)).or(self.names.first().map(String::as_str));
        let Some(name) = name.map(ToOwned::to_owned) else {
            self.selected = None;
            self.saved = Values::empty();
            self.edited = Values::empty();
            return Ok(());
        };
        let entry = file.context(&name).unwrap_or_default();
        self.has_entry = tray.get(&name).is_some();
        let (types, concurrency) = match tray.get(&name) {
            Some(saved) => (self.registered_of(&saved.types), saved.concurrency.to_string()),
            None => (self.registered_of(&self.registered), WorkerConfig::default().concurrency.to_string()),
        };
        self.saved = Values { server: entry.server, token: entry.token.unwrap_or_default(), types, concurrency };
        self.edited = self.saved.clone();
        self.selected = Some(Selected::Existing(name));
        Ok(())
    }

    /// The registered types among `names`, in the order the app registers them.
    fn registered_of(&self, names: &[impl AsRef<str>]) -> Vec<String> {
        let picked = |t: &&&str| names.iter().any(|n| n.as_ref() == **t);
        self.registered.iter().filter(picked).map(|t| t.to_string()).collect()
    }

    /// The names of the contexts in the config file as last read, with a new context not yet applied at the end.
    pub fn contexts(&self) -> Vec<String> {
        let mut names = self.names.clone();
        if let Some(Selected::New(name)) = &self.selected {
            names.push(name.clone());
        }
        names
    }

    /// The selected context, if any.
    pub fn selected(&self) -> Option<&str> {
        match self.selected.as_ref()? {
            Selected::Existing(name) | Selected::New(name) => Some(name),
        }
    }

    /// Reads the files again and shows the values of `name`; edits of the previous context are dropped.
    pub fn select(&mut self, name: &str) -> Result<(), String> {
        self.read(Some(name))
    }

    /// Selects a new context `name` with no server and every type: 1 to 64 characters from `A-Z a-z 0-9 . _ -`, and
    /// no name the config file has.
    pub fn add(&mut self, name: &str) -> Result<(), String> {
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        if name.is_empty() || name.chars().count() > MAX_NAME || !name.chars().all(allowed) {
            return Err(format!("a name has 1 to {MAX_NAME} characters from A-Z, a-z, 0-9, '.', '_' and '-'"));
        }
        if self.names.iter().any(|n| n == name) {
            return Err(format!("a context named {name} exists"));
        }
        let types = self.registered.iter().map(|t| t.to_string()).collect();
        let concurrency = WorkerConfig::default().concurrency.to_string();
        self.saved = Values { server: String::new(), token: String::new(), types, concurrency };
        self.edited = self.saved.clone();
        self.has_entry = false;
        self.selected = Some(Selected::New(name.to_owned()));
        Ok(())
    }

    /// Restores the values on disk: of the selected context, or of the previous one when the selected one is new.
    pub fn revert(&mut self) -> Result<(), String> {
        let wanted = match &self.selected {
            Some(Selected::Existing(name)) => Some(name.clone()),
            _ => None,
        };
        self.read(wanted.as_deref())
    }

    /// Says when `CICADA_SERVER` or `CICADA_TOKEN` in the tray's environment wins over the context's values.
    pub fn environment_note(&self) -> Option<String> {
        let set = |name: &str| self.env.get(name).is_some_and(|value| !value.trim().is_empty());
        let names: Vec<&str> = ["CICADA_SERVER", "CICADA_TOKEN"].into_iter().filter(|name| set(name)).collect();
        match names.as_slice() {
            [] => None,
            [name] => Some(format!("{name} is set and wins over this context")),
            _ => Some("CICADA_SERVER and CICADA_TOKEN are set and win over this context".to_owned()),
        }
    }

    /// The errors of the edited values, each with its field; empty when they are valid.
    pub fn validate(&self) -> Vec<(Field, String)> {
        let mut errors = Vec::new();
        if self.selected.is_none() {
            errors.push((Field::Context, "add a context first".to_owned()));
        }
        let server = self.edited.server.trim();
        if server.is_empty() {
            errors.push((Field::Server, "enter the server's URL".to_owned()));
        } else if let Err(error) = check_server_url(server) {
            errors.push((Field::Server, format!("{error:#}")));
        }
        match self.edited.types.as_slice() {
            [] => errors.push((Field::Types, "pick at least one type".to_owned())),
            [only] if (only == "all" || only == "none") && self.registered.len() > 1 => errors.push((
                Field::Types,
                format!("a type named {only} cannot run alone while others are registered; pick another with it"),
            )),
            _ => {}
        }
        if self.concurrency().is_none() {
            errors.push((Field::Concurrency, format!("enter a whole number from 1 through {MAX_CONCURRENCY}")));
        }
        errors
    }

    fn concurrency(&self) -> Option<usize> {
        self.edited.concurrency.trim().parse().ok().filter(|n| (1..=MAX_CONCURRENCY).contains(n))
    }

    /// Validates the values and writes what the user changed, holding the config file's lock for both writes: first
    /// the server, token, a new context and `current-context` into the config file, then the worker settings into
    /// `tray.toml` under its own lock. Fields the user left alone keep what other writers saved meanwhile; a context
    /// without a `tray.toml` entry gets the shown types and concurrency as its entry, so that what the window shows is
    /// what the worker runs. The name of the context to connect to when both are saved.
    pub fn apply(&mut self) -> Result<String, ApplyError> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(ApplyError::Invalid(errors));
        }
        let (name, new) = match self.selected.clone().expect("validated") {
            Selected::Existing(name) => (name, false),
            Selected::New(name) => (name, true),
        };
        let server =
            (new || self.edited.server.trim() != self.saved.server).then(|| self.edited.server.trim().to_owned());
        let token = (new || self.edited.token != self.saved.token).then(|| self.edited.token.clone());
        let make_current = self.current.as_deref() != Some(name.as_str());
        // Fields the user edited replace those on disk; the rest stay as another writer saved them. When the form
        // found no entry, the shown values become one, unless another writer has saved an entry by Apply.
        let types_edited = self.edited.types != self.saved.types;
        let concurrency_edited = self.edited.concurrency != self.saved.concurrency;
        let worker = (!self.has_entry || types_edited || concurrency_edited).then(|| WorkerSettings {
            types: self.edited.types.clone(),
            concurrency: self.concurrency().expect("valid"),
        });
        let env = self.env.clone();
        let outcome = ContextFile::update(&env, |file| {
            let tray = locate(file).map_err(anyhow::Error::msg)?;
            let connection = server.is_some() || token.is_some() || make_current;
            if connection {
                if new && file.context(&name).is_some() {
                    return Ok(Err(ApplyError::Invalid(vec![(
                        Field::Context,
                        format!("a context named {name} was added meanwhile; pick it or use another name"),
                    )])));
                }
                if server.is_some() || token.is_some() {
                    let entry = file.context(&name).unwrap_or_default();
                    let server = server.clone().unwrap_or(entry.server);
                    let token = token.clone().map_or(entry.token, |t| Some(t).filter(|t| !t.is_empty()));
                    file.set_server_and_token(&name, server, token);
                }
                if make_current {
                    file.use_context(&name)?;
                }
                file.save()?;
                self.connection_saved(&name);
            }
            if let Some(settings) = worker {
                if let Err(error) = TrayFile::update(&tray, |tray| {
                    tray.merge(&name, settings, types_edited, concurrency_edited);
                    Ok(())
                }) {
                    let saved = if connection { "connection saved; " } else { "" };
                    return Ok(Err(ApplyError::Partial(format!("{saved}worker settings not saved: {error:#}"))));
                }
                self.saved.types = self.edited.types.clone();
                self.saved.concurrency = self.edited.concurrency.clone();
                self.has_entry = true;
            }
            Ok(Ok(name.clone()))
        });
        outcome.unwrap_or_else(|error| Err(ApplyError::Failed(format!("{error:#}"))))
    }

    /// Marks the connection fields of `name` as on disk: an added context becomes an existing one and the current.
    fn connection_saved(&mut self, name: &str) {
        self.saved.server = self.edited.server.trim().to_owned();
        self.saved.token = self.edited.token.clone();
        if !self.names.iter().any(|n| n == name) {
            self.names.push(name.to_owned());
            self.names.sort();
        }
        self.current = Some(name.to_owned());
        self.selected = Some(Selected::Existing(name.to_owned()));
    }
}

impl Values {
    fn empty() -> Self {
        Self { server: String::new(), token: String::new(), types: Vec::new(), concurrency: String::new() }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cicada_app::ContextEntry;

    use super::*;
    use crate::tray_file::FILE;

    const REGISTERED: [&str; 2] = ["work", "other"];

    struct Files {
        dir: tempfile::TempDir,
    }

    impl Files {
        fn new(config: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("config.toml"), config).unwrap();
            Self { dir }
        }

        fn env(&self) -> Env {
            Env::from([("CICADA_CONFIG".to_owned(), self.config().to_str().unwrap().to_owned())])
        }

        fn config(&self) -> PathBuf {
            self.dir.path().join("config.toml")
        }

        fn tray(&self) -> PathBuf {
            self.dir.path().join(FILE)
        }

        fn contexts(&self) -> ContextFile {
            ContextFile::load(&self.env()).unwrap()
        }

        fn form(&self) -> SettingsForm {
            SettingsForm::load(&self.env(), &REGISTERED, None).unwrap()
        }

        fn worker(&self, context: &str) -> Option<WorkerSettings> {
            TrayFile::load(&self.tray()).unwrap().get(context).cloned()
        }
    }

    const PROD: &str = "current-context = \"prod\"\n\
                        [contexts.prod]\nserver = \"http://prod\"\ntoken = \"t0\"\nheaders = { X-Tenant = \"ops\" }\n";

    fn text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn it_loads_the_current_context_with_inherited_worker_settings() {
        let files = Files::new(PROD);
        let form = files.form();
        assert_eq!(form.selected(), Some("prod"));
        assert_eq!(form.contexts(), ["prod"]);
        assert_eq!((form.edited.server.as_str(), form.edited.token.as_str()), ("http://prod", "t0"));
        assert_eq!(form.edited.types, ["work", "other"]);
        assert_eq!(form.edited.concurrency, WorkerConfig::default().concurrency.to_string());
        assert!(form.validate().is_empty());
    }

    #[test]
    fn no_context_needs_an_add() {
        let files = Files::new("");
        let mut form = files.form();
        assert_eq!(form.selected(), None);
        assert_eq!(
            form.apply().unwrap_err(),
            ApplyError::Invalid(vec![
                (Field::Context, "add a context first".into()),
                (Field::Server, "enter the server's URL".into()),
                (Field::Types, "pick at least one type".into()),
                (Field::Concurrency, "enter a whole number from 1 through 1000".into()),
            ])
        );
    }

    /// Scenario: An invalid server.
    #[test]
    fn each_invalid_value_is_named_under_its_field_and_nothing_is_written() {
        let files = Files::new(PROD);
        let before = text(&files.config());
        let mut form = files.form();
        form.edited.server = "ftp://x".into();
        form.edited.types.clear();
        form.edited.concurrency = "1001".into();
        let Err(ApplyError::Invalid(errors)) = form.apply() else { panic!("invalid") };
        let fields: Vec<Field> = errors.iter().map(|(field, _)| *field).collect();
        assert_eq!(fields, [Field::Server, Field::Types, Field::Concurrency]);
        assert_eq!(errors[0].1, "the server URL must use http or https");
        for concurrency in ["0", "x", "-1", ""] {
            form.edited.concurrency = concurrency.into();
            assert!(form.validate().iter().any(|(field, _)| *field == Field::Concurrency), "{concurrency}");
        }
        for concurrency in ["1", " 1000 "] {
            form.edited.concurrency = concurrency.into();
            assert!(!form.validate().iter().any(|(field, _)| *field == Field::Concurrency), "{concurrency}");
        }
        assert_eq!(text(&files.config()), before);
        assert!(!files.tray().exists());
    }

    #[test]
    fn a_lone_type_named_all_or_none_is_refused_among_others() {
        let files = Files::new(PROD);
        for only in ["all", "none"] {
            let mut form = SettingsForm::load(&files.env(), &[only, "b"], None).unwrap();
            form.edited.types = vec![only.into()];
            assert_eq!(form.validate()[0].0, Field::Types);
            form.edited.types = vec![only.into(), "b".into()];
            assert!(form.validate().is_empty());
            let mut alone = SettingsForm::load(&files.env(), &[only], None).unwrap();
            alone.edited.types = vec![only.into()];
            assert!(alone.validate().is_empty(), "the only registered type");
        }
    }

    #[test]
    fn a_new_name_follows_the_rule_and_an_existing_odd_name_is_kept() {
        let files = Files::new("[contexts.\"odd name!\"]\nserver = \"http://odd\"\n");
        let mut form = files.form();
        assert_eq!(form.selected(), Some("odd name!"));
        for bad in ["", "has space", "slash/", &"x".repeat(65), "odd name!"] {
            assert!(form.add(bad).is_err(), "{bad:?}");
        }
        form.edited.server = "http://odd2".into();
        assert_eq!(form.apply(), Ok("odd name!".into()));
        assert_eq!(files.contexts().context("odd name!").unwrap().server, "http://odd2");
        form.add(&"x".repeat(64)).unwrap();
        form.add("a-Z_0.9").unwrap();
        assert_eq!(form.contexts(), ["odd name!", "a-Z_0.9"], "only the last added one");
    }

    #[test]
    fn revert_restores_the_values_on_disk() {
        let files = Files::new(PROD);
        let mut form = files.form();
        form.edited.server = "http://edited".into();
        form.edited.types = vec!["work".into()];
        form.revert().unwrap();
        assert_eq!(form.edited.server, "http://prod");
        assert_eq!(form.edited.types, ["work", "other"]);
        form.add("new").unwrap();
        form.revert().unwrap();
        assert_eq!((form.selected(), form.contexts()), (Some("prod"), vec!["prod".to_owned()]));
    }

    #[test]
    fn select_shows_the_values_of_another_context() {
        let files = Files::new(&format!("{PROD}[contexts.test]\nserver = \"http://test\"\n"));
        std::fs::write(files.tray(), "[contexts.test]\ntypes = [\"other\", \"gone\"]\nconcurrency = 3\n").unwrap();
        let mut form = files.form();
        form.select("test").unwrap();
        assert_eq!((form.edited.server.as_str(), form.edited.token.as_str()), ("http://test", ""));
        assert_eq!((form.edited.types.clone(), form.edited.concurrency.as_str()), (vec!["other".into()], "3"));
    }

    #[test]
    fn the_environment_note_names_the_variables_that_win() {
        let files = Files::new(PROD);
        let mut env = files.env();
        assert_eq!(SettingsForm::load(&env, &REGISTERED, None).unwrap().environment_note(), None);
        env.insert("CICADA_TOKEN".into(), "x".into());
        let note = SettingsForm::load(&env, &REGISTERED, None).unwrap().environment_note().unwrap();
        assert_eq!(note, "CICADA_TOKEN is set and wins over this context");
        env.insert("CICADA_SERVER".into(), "http://env".into());
        let note = SettingsForm::load(&env, &REGISTERED, None).unwrap().environment_note().unwrap();
        assert!(note.contains("CICADA_SERVER and CICADA_TOKEN"));
    }

    /// Scenario: Another writer meanwhile.
    #[test]
    fn apply_keeps_a_context_another_writer_added_and_sets_the_current_context() {
        let files = Files::new(&format!("{PROD}[contexts.test]\nserver = \"http://test\"\n"));
        let mut form = files.form();
        form.select("test").unwrap();
        form.edited.token = "t1".into();
        form.edited.types = vec!["work".into()];
        form.edited.concurrency = "4".into();
        ContextFile::update(&files.env(), |file| {
            file.set_context("staging", ContextEntry { server: "http://staging".into(), ..ContextEntry::default() });
            file.set_server_and_token("test", "http://moved".into(), None);
            Ok(())
        })
        .unwrap();

        assert_eq!(form.apply(), Ok("test".into()));
        let contexts = files.contexts();
        assert_eq!(contexts.names().collect::<Vec<_>>(), ["prod", "staging", "test"]);
        assert_eq!(contexts.current(), Some("test"));
        let test = contexts.context("test").unwrap();
        assert_eq!((test.server.as_str(), test.token.as_deref()), ("http://moved", Some("t1")), "server not edited");
        assert_eq!(contexts.context("prod").unwrap().headers["X-Tenant"], "ops");
        assert_eq!(files.worker("test"), Some(WorkerSettings { types: vec!["work".into()], concurrency: 4 }));
        assert_eq!(files.worker("prod"), None, "a context without changes inherits");
    }

    /// Scenario: All types over the environment, through the form.
    #[test]
    fn apply_saves_the_shown_defaults_as_an_entry_that_wins_over_the_environment() {
        let files = Files::new(PROD);
        let mut env = files.env();
        env.extend([("CICADA_TYPES".to_owned(), "work".to_owned()), ("CICADA_CONCURRENCY".to_owned(), "2".to_owned())]);
        let before = text(&files.config());
        let mut form = SettingsForm::load(&env, &REGISTERED, None).unwrap();
        assert_eq!(form.apply(), Ok("prod".into()));
        assert_eq!(text(&files.config()), before, "no connection field changed");
        let defaults = WorkerSettings { types: vec!["work".into(), "other".into()], concurrency: 16 };
        assert_eq!(files.worker("prod"), Some(defaults));
        let args = crate::tray_file::saved_args(&env, "prod", &REGISTERED).unwrap();
        assert_eq!(args, ["--types=all", "--concurrency=16"]);

        std::fs::remove_file(files.tray()).unwrap();
        std::fs::create_dir(files.tray()).unwrap(); // a second write would fail now
        assert_eq!(form.apply(), Ok("prod".into()), "an apply without changes writes nothing");
    }

    #[test]
    fn two_forms_opened_before_an_entry_existed_keep_both_edits() {
        let files = Files::new(PROD);
        let (mut types, mut concurrency) = (files.form(), files.form());
        concurrency.edited.concurrency = "9".into();
        assert_eq!(concurrency.apply(), Ok("prod".into()));
        types.edited.types = vec!["other".into()];
        assert_eq!(types.apply(), Ok("prod".into()));
        assert_eq!(files.worker("prod"), Some(WorkerSettings { types: vec!["other".into()], concurrency: 9 }));

        std::fs::remove_file(files.tray()).unwrap();
        let (mut untouched, mut other) = (files.form(), files.form());
        other.edited.concurrency = "5".into();
        assert_eq!(other.apply(), Ok("prod".into()));
        assert_eq!(untouched.apply(), Ok("prod".into()));
        let saved = files.worker("prod").unwrap();
        assert_eq!(saved.concurrency, 5, "an Apply without edits keeps an entry saved meanwhile");
    }

    #[test]
    fn two_forms_editing_different_worker_fields_keep_both() {
        let files = Files::new(PROD);
        std::fs::write(files.tray(), "[contexts.prod]\ntypes = [\"work\", \"other\"]\nconcurrency = 4\n").unwrap();
        let (mut types, mut concurrency) = (files.form(), files.form());
        concurrency.edited.concurrency = "9".into();
        assert_eq!(concurrency.apply(), Ok("prod".into()));
        types.edited.types = vec!["other".into()];
        assert_eq!(types.apply(), Ok("prod".into()));
        assert_eq!(files.worker("prod"), Some(WorkerSettings { types: vec!["other".into()], concurrency: 9 }));
    }

    #[test]
    fn apply_keeps_the_headers_and_clears_an_emptied_token() {
        let files = Files::new(PROD);
        let mut form = files.form();
        form.edited.token.clear();
        assert_eq!(form.apply(), Ok("prod".into()));
        let prod = files.contexts().context("prod").unwrap();
        assert_eq!((prod.token, prod.headers["X-Tenant"].as_str()), (None, "ops"));
        assert!(!text(&files.config()).contains("t0"));
    }

    #[test]
    fn an_add_refuses_a_name_another_writer_took_meanwhile() {
        let files = Files::new(PROD);
        let mut form = files.form();
        form.add("staging").unwrap();
        form.edited.server = "http://mine".into();
        ContextFile::update(&files.env(), |file| {
            file.set_context("staging", ContextEntry { server: "http://theirs".into(), ..ContextEntry::default() });
            Ok(())
        })
        .unwrap();
        let Err(ApplyError::Invalid(errors)) = form.apply() else { panic!("refused") };
        assert_eq!(errors[0].0, Field::Context);
        assert_eq!(files.contexts().context("staging").unwrap().server, "http://theirs");
    }

    #[test]
    fn a_failed_tray_write_is_reported_and_a_retry_after_an_add_keeps_a_config_edit_made_meanwhile() {
        let files = Files::new(PROD);
        let mut form = files.form();
        form.add("staging").unwrap();
        form.edited.server = "http://staging".into();
        form.edited.token = "ts".into();
        form.edited.types = vec!["other".into()];
        std::fs::create_dir(files.tray()).unwrap(); // a folder cannot be read or replaced as the tray's file

        let Err(ApplyError::Partial(error)) = form.apply() else { panic!("partial") };
        assert!(error.starts_with("connection saved; worker settings not saved: cannot read"), "{error}");
        let contexts = files.contexts();
        assert_eq!(
            (contexts.current(), contexts.context("staging").unwrap().token),
            (Some("staging"), Some("ts".into()))
        );

        std::fs::remove_dir(files.tray()).unwrap();
        ContextFile::update(&files.env(), |file| {
            file.set_server_and_token("staging", "http://edited".into(), Some("other".into()));
            file.use_context("prod")
        })
        .unwrap();
        assert_eq!(form.apply(), Ok("staging".into()), "no duplicate-name refusal");
        let contexts = files.contexts();
        let staging = contexts.context("staging").unwrap();
        assert_eq!((staging.server.as_str(), staging.token.as_deref()), ("http://edited", Some("other")));
        assert_eq!(contexts.current(), Some("prod"), "the current context set meanwhile is kept");
        assert_eq!(files.worker("staging"), Some(WorkerSettings { types: vec!["other".into()], concurrency: 16 }));
    }

    #[test]
    fn a_config_named_like_the_tray_file_cannot_be_loaded() {
        let files = Files::new(PROD);
        let env = Env::from([("CICADA_CONFIG".to_owned(), files.tray().to_str().unwrap().to_owned())]);
        assert!(SettingsForm::load(&env, &REGISTERED, None).err().unwrap().contains("CICADA_CONFIG"));
    }
}
