//! The one error type of the runtime.

use std::fmt;

/// An error of a [`ContainerRuntime`](crate::ContainerRuntime) call: its kind and a message from the backend.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct ContainerError {
    /// What went wrong, and with it whether a retry can help.
    pub kind: ErrorKind,
    /// The backend's description.
    pub message: String,
}

impl ContainerError {
    /// An error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    /// True only for [`ErrorKind::DaemonUnavailable`]. A caller may still retry the others, counting errors in a row.
    pub fn is_retryable(&self) -> bool {
        self.kind == ErrorKind::DaemonUnavailable
    }
}

/// The kind of a [`ContainerError`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The runtime could not be reached, or timed out; the only kind a retry is expected to cure.
    DaemonUnavailable,
    /// The call does not fit the current state: `start`, `copy_in`, `copy_out`, `logs` or `logs_since` on a missing
    /// container, `copy_in` on a container that is not `Created`, or `ensure_network` on an existing network with
    /// another `internal` or `isolated` flag.
    Conflict,
    /// The image does not exist, or could not be pulled.
    ImageNotFound,
    /// The runtime refused the spec or the arguments, such as a relative path for `copy_in` or `copy_out`, a
    /// `max_lines` of 0 for `logs_since` or a zero limit for `copy_out`.
    InvalidSpec,
    /// The runtime does not implement the call, or cannot do it for this container: the trait's default
    /// [`logs_since`], [`copy_out`], [`make_dirs`] and [`info`] of a runtime written before them, or [`logs`] and
    /// [`logs_since`] of a container whose log driver keeps none, or [`ensure_network`] on a runtime that cannot
    /// isolate the network. A retry cannot help.
    ///
    /// [`logs`]: crate::ContainerRuntime::logs
    /// [`logs_since`]: crate::ContainerRuntime::logs_since
    /// [`copy_out`]: crate::ContainerRuntime::copy_out
    /// [`make_dirs`]: crate::ContainerRuntime::make_dirs
    /// [`info`]: crate::ContainerRuntime::info
    /// [`ensure_network`]: crate::ContainerRuntime::ensure_network
    Unsupported,
    /// Anything else the backend reports.
    Other,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DaemonUnavailable => "daemon unavailable",
            Self::Conflict => "conflict",
            Self::ImageNotFound => "image not found",
            Self::InvalidSpec => "invalid spec",
            Self::Unsupported => "unsupported",
            Self::Other => "error",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unavailable_daemon_is_retryable() {
        assert!(ContainerError::new(ErrorKind::DaemonUnavailable, "no socket").is_retryable());
        let others = [ErrorKind::Conflict, ErrorKind::ImageNotFound, ErrorKind::InvalidSpec, ErrorKind::Unsupported];
        for kind in others.into_iter().chain([ErrorKind::Other]) {
            assert!(!ContainerError::new(kind, "x").is_retryable(), "{kind:?}");
        }
    }

    #[test]
    fn the_message_names_the_kind() {
        let error = ContainerError::new(ErrorKind::Conflict, "no container job-1");
        assert_eq!(error.to_string(), "conflict: no container job-1");
    }
}
