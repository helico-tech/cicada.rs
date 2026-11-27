//! Redaction of untrusted action responses before they reach the view.

use std::sync::LazyLock;

use base64::{Engine, engine::general_purpose::STANDARD};
use cicada_app::{ContextEntry, Env};
use cicada_client::ClientError;
use http::StatusCode;
use percent_encoding::percent_decode_str;
use regex::Regex;
use reqwest::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Alert {
    pub title: String,
    pub detail: Option<String>,
}

impl Alert {
    fn http(status: u16) -> Self {
        Self { title: format!("HTTP {status}"), detail: None }
    }
}

/// The credentials configured for the client. Keep them out of view values and out of Debug.
#[derive(Default)]
pub(crate) struct Redactions {
    secrets: Vec<String>,
    uncertain: bool,
}

impl Redactions {
    pub(crate) fn for_context(env: &Env, context: Option<&ContextEntry>, uncertain: bool) -> Self {
        let mut redactions = Self { secrets: Vec::new(), uncertain };
        let token = env
            .get("CICADA_TOKEN")
            .filter(|value| !value.is_empty())
            .or_else(|| context.and_then(|entry| entry.token.as_ref()));
        redactions.secrets.extend(token.cloned());
        if let Some(entry) = context {
            for (name, value) in &entry.headers {
                if credential_header(name) || value.trim().chars().take(4).count() == 4 {
                    redactions.secrets.push(value.trim().to_owned());
                }
                if name.eq_ignore_ascii_case("authorization") {
                    redactions.authorization(value);
                }
            }
        }
        let server =
            env.get("CICADA_SERVER").filter(|value| !value.is_empty()).or_else(|| context.map(|entry| &entry.server));
        if let Some(url) = server.and_then(|value| Url::parse(value).ok()) {
            let user = url.username();
            let password = url.password();
            if let Some(password) = password {
                redactions.secrets.push(password.to_owned());
                redactions.secrets.push(percent_decode_str(password).decode_utf8_lossy().into_owned());
            }
            if (!user.is_empty() || password.is_some())
                && let Ok(user) = percent_decode_str(user).decode_utf8()
            {
                let password = password.and_then(|value| percent_decode_str(value).decode_utf8().ok());
                redactions.secrets.push(STANDARD.encode(format!("{user}:{}", password.unwrap_or_default())));
            }
            redactions.secrets.extend(url.query_pairs().map(|(_, value)| value.into_owned()));
        }
        for secret in &mut redactions.secrets {
            secret.retain(|c| !c.is_control());
        }
        redactions.secrets.retain(|value| !value.trim().is_empty());
        redactions.secrets.sort_by_key(|value| (std::cmp::Reverse(value.len()), value.clone()));
        redactions.secrets.dedup();
        redactions
    }

    fn authorization(&mut self, value: &str) {
        if let Some((scheme, token)) = value.split_once(' ') {
            let token = token.trim();
            self.secrets.push(token.to_owned());
            if scheme.eq_ignore_ascii_case("basic")
                && let Ok(bytes) = STANDARD.decode(token)
                && let Ok(decoded) = String::from_utf8(bytes)
                && let Some((_, password)) = decoded.split_once(':')
            {
                self.secrets.push(password.to_owned());
            }
        }
    }

    pub(super) fn alert(&self, error: ClientError) -> Alert {
        match error {
            ClientError::Api(problem) => {
                let reason = StatusCode::from_u16(problem.status).ok().and_then(|status| status.canonical_reason());
                // The client synthesizes this same shape for non-problem bodies, putting the raw body in detail.
                // A canonical about:blank is indistinguishable, so use the status for both.
                if self.uncertain
                    || (problem.problem_type == "about:blank"
                        && problem.instance.is_none()
                        && problem.title == reason.unwrap_or("Unknown status"))
                {
                    return Alert::http(problem.status);
                }
                Alert {
                    title: self.part(&problem.title).unwrap_or_else(|| format!("HTTP {}", problem.status)),
                    detail: problem.detail.as_deref().and_then(|detail| self.part(detail)),
                }
            }
            ClientError::Http(error) => Alert {
                title: self
                    .part(&format!("HTTP request failed: {}", error.without_url()))
                    .unwrap_or_else(|| "HTTP request failed".into()),
                detail: None,
            },
            // Purge is the only write that decodes a body, and it only decodes a 200 response.
            ClientError::Body(_) => Alert::http(200),
            ClientError::Invalid(error) => {
                Alert { title: self.part(&error).unwrap_or_else(|| "Could not make the request".into()), detail: None }
            }
        }
    }

    fn part(&self, text: &str) -> Option<String> {
        if self.uncertain {
            return None;
        }
        static AUTH: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)\b(?:basic|bearer) +[^\s,;"'<>]+"#).expect("authorization pattern"));
        static USERINFO: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)(https?://)[^\s/<>"?#]*@"#).expect("URL userinfo pattern"));
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        let text = AUTH.replace_all(&text, "");
        let mut text = USERINFO.replace_all(&text, "${1}").into_owned();
        for secret in &self.secrets {
            text = text.replace(secret, "");
        }
        let text = text.trim();
        (!text.is_empty()).then(|| text.chars().take(500).collect())
    }
}

fn credential_header(name: &str) -> bool {
    name.to_ascii_lowercase()
        .split(['-', '_'])
        .any(|word| matches!(word, "authorization" | "cookie" | "token" | "key" | "secret"))
}
