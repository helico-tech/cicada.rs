//! Database locations: libpq-style and JDBC URLs, normalised to what sqlx understands.

use std::fmt;
use std::str::FromStr;

use cicada::store::StoreError;
use sqlx::postgres::PgConnectOptions;

/// A database location normalised to what sqlx understands: a libpq-style URL without credentials, plus the
/// credentials to connect with.
///
/// Accepted forms:
/// - a libpq-style URL as found in `DATABASE_URL` (Heroku, Render, Fly, docker-compose setups):
///   `postgres://user:password@host:5432/db?sslmode=require` or `postgresql://…`. User and password are taken from
///   the user-info part (percent-decoded); host, port, database and query parameters are kept. Query parameters are
///   passed as they are, so only those sqlx knows work (`sslmode`, `sslrootcert`, `application_name`, `options`, …).
/// - a JDBC URL as used by the Kotlin implementation, `jdbc:postgresql://host:5432/db?sslmode=require`. Its `user`
///   and `password` parameters become the credentials, `ApplicationName` becomes `application_name`, and `ssl=true`
///   becomes `sslmode=verify-full` (pgjdbc's meaning) unless `sslmode` is given; other parameters are kept.
///
/// Several comma-separated hosts are not supported. Explicit credentials win over those in the URL.
#[derive(Clone, PartialEq, Eq)]
pub struct PostgresConnection {
    /// `postgres://host[:port]/db[?params]`, without credentials.
    pub url: String,
    /// The user to connect as, or `None` for sqlx's default (`PGUSER`, else the OS user).
    pub user: Option<String>,
    /// The password, or `None`.
    pub password: Option<String>,
}

impl fmt::Debug for PostgresConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresConnection")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

impl PostgresConnection {
    /// Parses `url` (see [`PostgresConnection`]); `user` and `password`, when given, replace the URL's own. Fails with
    /// [`StoreError::Invalid`] when `url` is neither a `jdbc:postgresql:` nor a `postgres(ql)://` URL; the message never
    /// contains the password.
    pub fn parse(url: &str, user: Option<&str>, password: Option<&str>) -> Result<Self, StoreError> {
        let trimmed = url.trim();
        let parsed = if let Some(rest) = trimmed.strip_prefix("jdbc:postgresql:") {
            // `jdbc:postgresql:db` names a database on localhost.
            let rest = if rest.starts_with("//") { rest.to_owned() } else { format!("//localhost/{rest}") };
            parse_url(trimmed, &rest, true)?
        } else {
            let (scheme, rest) = trimmed.split_once("://").unwrap_or(("", ""));
            if scheme != "postgres" && scheme != "postgresql" {
                return Err(StoreError::Invalid(format!(
                    "Not a PostgreSQL URL (expected jdbc:postgresql://…, postgres://… or postgresql://…): {}",
                    redact(trimmed)
                )));
            }
            parse_url(trimmed, &format!("//{rest}"), false)?
        };
        Ok(Self {
            url: parsed.url,
            user: user.map(str::to_owned).or(parsed.user),
            password: password.map(str::to_owned).or(parsed.password),
        })
    }

    /// The sqlx connect options of this location.
    pub fn connect_options(&self) -> Result<PgConnectOptions, StoreError> {
        let mut options = PgConnectOptions::from_str(&self.url)
            .map_err(|e| StoreError::Invalid(format!("Malformed PostgreSQL URL {}: {e}", self.url)))?;
        if let Some(user) = &self.user {
            options = options.username(user);
        }
        if let Some(password) = &self.password {
            options = options.password(password);
        }
        Ok(options)
    }
}

struct Parsed {
    url: String,
    user: Option<String>,
    password: Option<String>,
}

/// Parses `//[user[:password]@]host[:port][/path][?query]`; `original` is the whole URL, for error messages.
fn parse_url(original: &str, rest: &str, jdbc: bool) -> Result<Parsed, StoreError> {
    let malformed = || StoreError::Invalid(format!("Malformed PostgreSQL URL: {}", redact(original)));
    let without_host = || StoreError::Invalid(format!("PostgreSQL URL without host: {}", redact(original)));
    let rest = rest.strip_prefix("//").ok_or_else(malformed)?;
    let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
    let (before_query, query) = match rest.split_once('?') {
        Some((before, query)) => (before, Some(query)),
        None => (rest, None),
    };
    let (authority, path) = match before_query.find('/') {
        Some(i) => (&before_query[..i], &before_query[i..]),
        None => (before_query, "/"),
    };
    let (user_info, host_port) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port) = if let Some(v6) = host_port.strip_prefix('[') {
        let (address, after) = v6.split_once(']').ok_or_else(malformed)?;
        (format!("[{address}]"), after.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (host_port.to_owned(), None),
        }
    };
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]:%".contains(c)) {
        return Err(without_host());
    }
    let port = match port {
        Some(p) if !p.is_empty() => Some(p.parse::<u16>().map_err(|_| malformed())?),
        _ => None,
    };
    let mut user = None;
    let mut password = None;
    if let Some(info) = user_info {
        let (u, p) = match info.split_once(':') {
            Some((u, p)) => (u, Some(p)),
            None => (info, None),
        };
        user = Some(decode(u).ok_or_else(malformed)?).filter(|u| !u.is_empty());
        password = p.map(|p| decode(p).ok_or_else(malformed)).transpose()?;
    }
    let mut params: Vec<String> = Vec::new();
    if let Some(query) = query {
        let pairs: Vec<(&str, &str)> =
            query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).collect();
        let has_sslmode = pairs.iter().any(|(k, _)| *k == "sslmode");
        for (key, value) in pairs {
            match key {
                "user" if jdbc => user = user.or(Some(decode(value).ok_or_else(malformed)?)),
                "password" if jdbc => password = password.or(Some(decode(value).ok_or_else(malformed)?)),
                "ApplicationName" if jdbc => params.push(format!("application_name={value}")),
                "ssl" if jdbc && !has_sslmode => match value {
                    "" | "true" => params.push("sslmode=verify-full".to_owned()),
                    "false" => params.push("sslmode=disable".to_owned()),
                    _ => return Err(malformed()),
                },
                "ssl" if jdbc => {}
                _ => params.push(if value.is_empty() { key.to_owned() } else { format!("{key}={value}") }),
            }
        }
    }
    let mut url = format!("postgres://{host}");
    if let Some(port) = port {
        url.push_str(&format!(":{port}"));
    }
    url.push_str(path);
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    Ok(Parsed { url, user, password })
}

/// Percent-decodes `text` (a `+` stays a `+`); `None` when it is malformed or not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Removes the password from a URL for error messages.
fn redact(url: &str) -> String {
    let Some(start) = url.find("://").map(|i| i + 3) else { return url.to_owned() };
    let authority_end = url[start..].find(['/', '?', '#']).map_or(url.len(), |i| start + i);
    let authority = &url[start..authority_end];
    match authority.rfind('@') {
        Some(at) => match authority[..at].split_once(':') {
            Some((user, _)) => format!("{}{user}:***{}", &url[..start], &url[start + at..]),
            None => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(url: &str, user: Option<&str>, password: Option<&str>) -> PostgresConnection {
        PostgresConnection { url: url.to_owned(), user: user.map(str::to_owned), password: password.map(str::to_owned) }
    }

    fn parse(url: &str) -> PostgresConnection {
        PostgresConnection::parse(url, None, None).unwrap()
    }

    #[test]
    fn a_jdbc_url_becomes_a_libpq_url() {
        assert_eq!(
            PostgresConnection::parse("jdbc:postgresql://db:5432/actors?sslmode=require", Some("u"), Some("p"))
                .unwrap(),
            conn("postgres://db:5432/actors?sslmode=require", Some("u"), Some("p")),
        );
        assert_eq!(parse(" jdbc:postgresql://db/actors "), conn("postgres://db/actors", None, None));
        assert_eq!(
            parse("jdbc:postgresql://db/actors?user=app&password=s%40cret&ApplicationName=x&ssl=true"),
            conn("postgres://db/actors?application_name=x&sslmode=verify-full", Some("app"), Some("s@cret")),
        );
        assert_eq!(
            parse("jdbc:postgresql://db/a?ssl=true&sslmode=disable"),
            conn("postgres://db/a?sslmode=disable", None, None)
        );
        assert_eq!(parse("jdbc:postgresql:actors"), conn("postgres://localhost/actors", None, None));
    }

    #[test]
    fn a_libpq_url_keeps_its_location_and_yields_its_credentials() {
        assert_eq!(
            parse("postgres://app:s3cr%40t%3A%2Fx@db.example:6543/orders?sslmode=require"),
            conn("postgres://db.example:6543/orders?sslmode=require", Some("app"), Some("s3cr@t:/x")),
        );
        assert_eq!(
            parse("postgresql://actors@localhost/actors"),
            conn("postgres://localhost/actors", Some("actors"), None)
        );
        assert_eq!(parse("postgres://h"), conn("postgres://h/", None, None));
        assert_eq!(parse("postgres://a:b+c@h/d"), conn("postgres://h/d", Some("a"), Some("b+c")));
        assert_eq!(parse("postgres://a@[::1]:5433/d"), conn("postgres://[::1]:5433/d", Some("a"), None));
    }

    #[test]
    fn explicit_credentials_win_over_the_urls() {
        assert_eq!(
            PostgresConnection::parse("postgres://app:secret@db/actors", Some("other"), Some("pw")).unwrap(),
            conn("postgres://db/actors", Some("other"), Some("pw")),
        );
    }

    #[test]
    fn other_urls_are_refused_without_leaking_the_password() {
        let message = |url: &str| match PostgresConnection::parse(url, None, None) {
            Err(StoreError::Invalid(m)) => m,
            other => panic!("expected Invalid for {url}, got {other:?}"),
        };
        assert!(!message("mysql://app:secret@db/actors").contains("secret"));
        assert!(message("mysql://app:secret@db/actors").contains("app:***@db"));
        message("actors.db");
        assert!(!message("postgres://a:secret@h1,h2/db").contains("secret"));
        message("postgres://h:notaport/db");
        assert!(format!("{:?}", conn("x", Some("u"), Some("secret"))).contains("***"));
        assert!(!format!("{:?}", conn("x", Some("u"), Some("secret"))).contains("secret"));
    }

    #[test]
    fn connect_options_carry_the_credentials() {
        let options = parse("postgres://app:pw@db:6543/orders").connect_options().unwrap();
        assert_eq!(options.get_host(), "db");
        assert_eq!(options.get_port(), 6543);
        assert_eq!(options.get_username(), "app");
        assert_eq!(options.get_database(), Some("orders"));
    }
}
