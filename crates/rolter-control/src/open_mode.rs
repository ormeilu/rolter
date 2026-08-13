//! Open mode: the control plane running with no `ROLTER_ADMIN_TOKEN`.
//!
//! With no admin token configured the [`Principal`](crate::rbac::Principal)
//! extractor short-circuits to `Superadmin` for every request, so anyone who
//! can reach the port has full administrative access — create and delete
//! providers, read and rotate keys, change routing, alter budgets. There is no
//! authentication step to fail.
//!
//! That is a reasonable local-development affordance and a bad default for
//! anything else, so this module makes it three things it previously was not:
//!
//! 1. **bounded** — open mode is refused outright on a non-loopback bind unless
//!    the operator passes `--allow-open-mode`, so it cannot escape the
//!    developer's machine by omission;
//! 2. **loud** — every start in open mode logs a warning naming exactly what is
//!    unprotected, rather than the single line that was easy to scroll past;
//! 3. **visible** — the decision rides into the dashboard's runtime config
//!    block, which renders a persistent banner while the control plane is
//!    unauthenticated.
//!
//! The evaluation itself is a pure function over "is a token set", "what are we
//! binding", and "did the operator acknowledge it", so the refusal is unit
//! tested without opening a socket.

use std::net::SocketAddr;

/// What the control plane decided about its own authentication before binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// an admin token is configured; the management API is gated
    Closed,
    /// no admin token, but every listener is on loopback — the supported
    /// zero-cred local-dev shape
    OpenLoopback,
    /// no admin token on a non-loopback listener, explicitly acknowledged with
    /// `--allow-open-mode`. Allowed because the operator asked for it by name,
    /// and warned about on every start
    OpenAcknowledged,
}

impl OpenMode {
    /// Whether the management API is currently unauthenticated. Drives both the
    /// startup warning and the dashboard banner.
    pub(crate) fn is_open(self) -> bool {
        !matches!(self, OpenMode::Closed)
    }
}

/// Refusal to start: open mode was reachable from off the machine and nothing
/// said that was intended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenModeRefused {
    /// the listener that would have been exposed
    pub addr: SocketAddr,
}

impl std::fmt::Display for OpenModeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refusing to start: no ROLTER_ADMIN_TOKEN is set, so the management API and \
             /internal/snapshot would accept every request as superadmin — and {} is not a \
             loopback address, so that is reachable from off this machine. Set \
             ROLTER_ADMIN_TOKEN to close it, bind to 127.0.0.1 for local development, or pass \
             --allow-open-mode (ROLTER_ALLOW_OPEN_MODE=1) if an unauthenticated control plane \
             on this interface is genuinely what you want",
            self.addr
        )
    }
}

impl std::error::Error for OpenModeRefused {}

/// Decide whether this process may bind at all, given its credential posture.
///
/// `addrs` is every listener the process is about to open — the public API and,
/// when `--internal-addr` splits it out, the internal channel that carries
/// decrypted provider credentials. A single non-loopback listener is enough to
/// refuse: it does not matter which socket the unauthenticated access arrives
/// on.
pub(crate) fn evaluate(
    admin_token_set: bool,
    addrs: &[SocketAddr],
    acknowledged: bool,
) -> Result<OpenMode, OpenModeRefused> {
    if admin_token_set {
        return Ok(OpenMode::Closed);
    }
    // `is_loopback` is false for the unspecified address (0.0.0.0 / ::), which
    // is the case that matters most: binding every interface is how open mode
    // escapes in practice
    let exposed = addrs.iter().find(|addr| !addr.ip().is_loopback());
    match (exposed, acknowledged) {
        (None, _) => Ok(OpenMode::OpenLoopback),
        (Some(_), true) => Ok(OpenMode::OpenAcknowledged),
        (Some(addr), false) => Err(OpenModeRefused { addr: *addr }),
    }
}

/// Log what open mode leaves unprotected, once, at startup.
///
/// Deliberately one warning per start rather than a line the operator can
/// mistake for routine: it names the endpoints, not just the missing variable.
pub(crate) fn warn(mode: OpenMode, addrs: &[SocketAddr]) {
    match mode {
        OpenMode::Closed => {}
        OpenMode::OpenLoopback => tracing::warn!(
            "ROLTER_ADMIN_TOKEN is unset: the management API (/api/v1/*) and \
             /internal/snapshot accept every request as superadmin — providers, virtual keys, \
             routing and budgets are all writable without a credential. This is allowed here \
             only because every listener is on loopback; set ROLTER_ADMIN_TOKEN before binding \
             any other interface"
        ),
        OpenMode::OpenAcknowledged => {
            let bound = addrs
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!(
                addrs = %bound,
                "SERVING AN UNAUTHENTICATED CONTROL PLANE ON A NON-LOOPBACK ADDRESS. \
                 --allow-open-mode was passed, so the management API (/api/v1/*) and \
                 /internal/snapshot accept every request as superadmin from anywhere that can \
                 reach these listeners: providers, virtual keys, routing and budgets are all \
                 readable and writable with no credential. Set ROLTER_ADMIN_TOKEN to close this"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_configured_admin_token_closes_the_control_plane_on_any_bind() {
        for bind in ["127.0.0.1:4001", "0.0.0.0:4001", "10.0.0.5:4001"] {
            assert_eq!(
                evaluate(true, &[addr(bind)], false),
                Ok(OpenMode::Closed),
                "{bind}"
            );
        }
    }

    #[test]
    fn open_mode_is_allowed_on_loopback() {
        assert_eq!(
            evaluate(false, &[addr("127.0.0.1:4001")], false),
            Ok(OpenMode::OpenLoopback)
        );
        assert_eq!(
            evaluate(false, &[addr("[::1]:4001")], false),
            Ok(OpenMode::OpenLoopback)
        );
    }

    #[test]
    fn open_mode_is_refused_on_a_non_loopback_bind() {
        // the unspecified address is the one that matters: it is how open mode
        // escapes the developer's machine without anyone choosing an interface
        let refused = evaluate(false, &[addr("0.0.0.0:4001")], false).unwrap_err();
        assert_eq!(refused.addr, addr("0.0.0.0:4001"));
        assert!(
            evaluate(false, &[addr("[::]:4001")], false).is_err(),
            "the ipv6 unspecified address is equally exposed"
        );
        assert!(evaluate(false, &[addr("10.0.0.5:4001")], false).is_err());
    }

    #[test]
    fn an_exposed_internal_listener_alone_is_enough_to_refuse() {
        // the public api on loopback is not reassuring when the credential
        // channel is bound to the world
        let refused = evaluate(
            false,
            &[addr("127.0.0.1:4001"), addr("0.0.0.0:4002")],
            false,
        )
        .unwrap_err();
        assert_eq!(refused.addr, addr("0.0.0.0:4002"));
    }

    #[test]
    fn acknowledging_open_mode_permits_a_non_loopback_bind() {
        assert_eq!(
            evaluate(false, &[addr("0.0.0.0:4001")], true),
            Ok(OpenMode::OpenAcknowledged)
        );
    }

    #[test]
    fn acknowledgement_does_not_invent_open_mode_when_a_token_is_set() {
        assert_eq!(
            evaluate(true, &[addr("0.0.0.0:4001")], true),
            Ok(OpenMode::Closed)
        );
    }

    #[test]
    fn only_the_closed_state_reports_itself_authenticated() {
        assert!(!OpenMode::Closed.is_open());
        assert!(OpenMode::OpenLoopback.is_open());
        assert!(OpenMode::OpenAcknowledged.is_open());
    }

    /// A container sets `ROLTER_ALLOW_OPEN_MODE=1` at least as readily as
    /// `=true`, and clap's derived bool parser accepts only the latter — which
    /// exited the control plane at startup ("invalid value '1'") rather than
    /// acknowledging open mode. The spelling must not be load-bearing.
    #[test]
    fn the_acknowledgement_accepts_the_truthy_spellings_env_vars_get() {
        use clap::builder::TypedValueParser;
        use clap::CommandFactory;

        let parser = clap::builder::FalseyValueParser::new();
        let command = crate::Args::command();
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("yes", true),
            ("on", true),
            ("0", false),
            ("false", false),
            ("", false),
        ] {
            let parsed = parser
                .parse_ref(&command, None, std::ffi::OsStr::new(value))
                .expect("every string is a valid falsey value");
            assert_eq!(parsed, expected, "ROLTER_ALLOW_OPEN_MODE={value:?}");
        }
    }

    /// The flag still works as a flag, taking no value on the command line.
    #[test]
    fn the_acknowledgement_is_still_a_bare_flag() {
        use clap::Parser;

        let args = crate::Args::parse_from(["rolter-control", "--allow-open-mode"]);
        assert!(args.allow_open_mode);
        assert!(!crate::Args::parse_from(["rolter-control"]).allow_open_mode);
    }

    #[test]
    fn the_refusal_names_the_variable_that_closes_it() {
        let msg = OpenModeRefused {
            addr: addr("0.0.0.0:4001"),
        }
        .to_string();
        assert!(msg.contains("ROLTER_ADMIN_TOKEN"), "{msg}");
        assert!(msg.contains("--allow-open-mode"), "{msg}");
        assert!(msg.contains("0.0.0.0:4001"), "{msg}");
    }
}
