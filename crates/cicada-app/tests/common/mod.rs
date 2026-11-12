//! What the tests of the command line and of `serve` and `worker` share.

#![allow(dead_code)] // each test crate uses a part

use std::ffi::OsString;
use std::io::Write;
use std::sync::{Arc, Mutex};

use cicada_app::{App, AppError, Env};

/// An output that a test reads while the command still writes to it.
#[derive(Debug, Clone, Default)]
pub struct Out(Arc<Mutex<Vec<u8>>>);

impl Out {
    /// What was written so far.
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `args` as the arguments of a process.
pub fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// An environment whose config file does not exist, so the user's own config never leaks into a test.
pub fn no_config() -> Env {
    Env::from([("CICADA_CONFIG".to_owned(), "/nonexistent/cicada/config.toml".to_owned())])
}

/// Runs `app` with `args` (the name of the program first) and `env`, and no stop; its output, or the error.
pub async fn run(app: App, env: &Env, list: &[&str]) -> Result<String, AppError> {
    let out = Out::default();
    app.run_with(args(list), env.clone(), out.clone(), std::future::pending()).await?;
    Ok(out.text())
}
