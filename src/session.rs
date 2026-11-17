//! What says a request came from the window keyhold opened.
//!
//! keyhold binds to loopback, and loopback is not a boundary: every process running as this user
//! can reach the port, and a web page in any tab can try to talk to it. So the HTTP surface starts
//! from "no", and a session is what turns it into "yes".
//!
//! How a session is handed over, and why:
//!
//!  - **The address carries a one-time code, never the session.** It lives for thirty seconds, is
//!    good for one exchange, and only one is outstanding at a time. Whatever keeps a copy of the
//!    address (the opener's command line, a log, the browser's history) keeps something already
//!    spent.
//!  - **The code never goes on a command line.** It is written into a small page in the state
//!    directory (`0600` inside `0700`), and the browser is handed the path of that page. Any user of
//!    the machine can read another process's command line; only this user can read that file.
//!  - **The session is an `HttpOnly` cookie**, so the page's own script never holds it. A cookie is
//!    attached by the browser on its own, which is why it is never enough by itself: every request
//!    must also carry a custom header (which another site cannot add without a CORS answer keyhold
//!    never gives) and come from this origin.
//!  - **A session belongs to one origin**, scheme, name and port. `127.0.0.1`, `localhost` and
//!    `keyhold.localhost` are different sites to a browser and each makes its own exchange.
//!  - **A code presented twice is an alarm.** Either something read it before the browser did and
//!    the browser is the second, or the reverse; both mean somebody else holds a session, and the
//!    one the code created is revoked.
//!  - **Unlocking replaces the session.** A browser that restores its tabs also restores session
//!    cookies, onto disk; after the next unlock, a restored copy opens nothing.
//!
//! Everything here lives in memory and dies with the process. Values are compared in constant time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::RngCore;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32 bytes of entropy, URL-safe, no padding.
const TOKEN_BYTES: usize = 32;

/// How long a code in an address is good for. The browser redeems it within a second; the rest is
/// slack for a slow machine.
pub const CODE_LIFETIME: Duration = Duration::from_secs(30);

/// Wrong codes tolerated before the outstanding one is thrown away.
const WRONG_CODES: u32 = 5;

/// Codes already exchanged that are still remembered, to recognise one presented again.
const SPENT_KEPT: usize = 16;

/// The header every request to the API must carry. Its value does not matter: its presence makes a
/// cross-origin request need a preflight, which keyhold never answers.
pub const MARK_HEADER: &str = "x-keyhold";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionToken {
    value: String,
}

impl SessionToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; TOKEN_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut bytes);

        let value = base64_url(&bytes);
        bytes.zeroize();

        Self { value }
    }

    /// Only for a `Set-Cookie` header or the page that redeems a code. Never logged, never persisted.
    pub fn expose(&self) -> &str {
        &self.value
    }

    pub fn matches(&self, candidate: &str) -> bool {
        // Length is not secret — a mismatch there is answered immediately — but the bytes are.
        candidate.len() == self.value.len() && candidate.as_bytes().ct_eq(self.value.as_bytes()).into()
    }
}

/// What an exchange of a code came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Exchange {
    /// A new session for the origin, to be set as the cookie.
    Granted(String),
    /// This code was already exchanged. The session it created has been revoked.
    Replayed,
    /// Not a code this process has outstanding, or one that expired.
    Refused,
}

struct Pending {
    code: SessionToken,
    issued: Instant,
    wrong: u32,
}

struct Spent {
    code: SessionToken,
    origin: String,
}

#[derive(Default)]
struct Inner {
    pending: Option<Pending>,
    spent: Vec<Spent>,
    sessions: HashMap<String, SessionToken>,
}

/// Codes, sessions and the cookie that carries them, for one running keyhold.
pub struct SessionGate {
    port: u16,
    secure: bool,
    inner: Mutex<Inner>,
}

impl SessionGate {
    /// `port` names the cookie, so two keyholds on one machine never overwrite each other's;
    /// `secure` is whether the interface is served over HTTPS.
    pub fn new(port: u16, secure: bool) -> Self {
        Self { port, secure, inner: Mutex::new(Inner::default()) }
    }

    pub fn secure(&self) -> bool {
        self.secure
    }

    /// A fresh code for an address, replacing any that was outstanding.
    pub fn issue_code(&self) -> String {
        let code = SessionToken::generate();
        let exposed = code.expose().to_string();

        self.inner.lock().unwrap().pending = Some(Pending { code, issued: Instant::now(), wrong: 0 });

        exposed
    }

    /// Trade a code for a session on `origin`.
    pub fn exchange(&self, code: &str, origin: &str) -> Exchange {
        self.exchange_at(code, origin, Instant::now())
    }

    fn exchange_at(&self, code: &str, origin: &str, now: Instant) -> Exchange {
        let mut inner = self.inner.lock().unwrap();

        let current = inner
            .pending
            .as_ref()
            .map(|pending| pending.code.matches(code) && now.duration_since(pending.issued) <= CODE_LIFETIME);

        if current == Some(true) {
            let pending = inner.pending.take().expect("checked above");
            let session = SessionToken::generate();
            let value = session.expose().to_string();

            inner.sessions.insert(origin.to_string(), session);
            inner.spent.push(Spent { code: SessionToken { value: pending.code.expose().to_string() }, origin: origin.to_string() });

            let excess = inner.spent.len().saturating_sub(SPENT_KEPT);
            inner.spent.drain(..excess);

            return Exchange::Granted(value);
        }

        if let Some(index) = inner.spent.iter().position(|spent| spent.code.matches(code)) {
            let spent = inner.spent.remove(index);
            inner.sessions.remove(&spent.origin);

            return Exchange::Replayed;
        }

        if let Some(pending) = inner.pending.as_mut() {
            pending.wrong += 1;

            if pending.wrong >= WRONG_CODES || now.duration_since(pending.issued) > CODE_LIFETIME {
                inner.pending = None;
            }
        }

        Exchange::Refused
    }

    /// Whether `presented` is the session of `origin`.
    pub fn check(&self, origin: &str, presented: &str) -> bool {
        self.inner.lock().unwrap().sessions.get(origin).map(|session| session.matches(presented)).unwrap_or(false)
    }

    /// A new session for `origin`, replacing every session this process has handed out.
    ///
    /// Called on unlock. Tabs on the same origin share the cookie the answer sets; a tab on another
    /// origin has to be opened again, which is the price of a restored cookie being worthless.
    pub fn renew(&self, origin: &str) -> String {
        let session = SessionToken::generate();
        let value = session.expose().to_string();

        let mut inner = self.inner.lock().unwrap();
        inner.sessions.clear();
        inner.sessions.insert(origin.to_string(), session);

        value
    }

    /// The cookie's name. `__Host-` over HTTPS, where the browser then insists on `Secure`, `Path=/`
    /// and no `Domain`; plain otherwise, because a browser refuses `Secure` from an `http://` name
    /// that is not loopback.
    pub fn cookie_name(&self) -> String {
        if self.secure {
            format!("__Host-keyhold_{}", self.port)
        } else {
            format!("keyhold_{}", self.port)
        }
    }

    /// The `Set-Cookie` value for a session. No `Expires`: it is a session cookie, and the session
    /// behind it dies with this process or the next unlock anyway.
    pub fn set_cookie(&self, value: &str) -> String {
        let secure = if self.secure { "; Secure" } else { "" };

        format!("{}={value}; Path=/; HttpOnly; SameSite=Strict{secure}", self.cookie_name())
    }

    /// This keyhold's session out of a `Cookie` header, if it carries one.
    pub fn from_cookie_header<'a>(&self, header: &'a str) -> Option<&'a str> {
        let name = self.cookie_name();

        header.split(';').map(str::trim).find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then_some(value)
        })
    }
}

/// URL-safe base64 without padding, written out rather than pulled in as a dependency.
///
/// A dependency in the path that handles key-adjacent bytes is a dependency whose next release can
/// read them. This is twenty lines and has no reason to ever change.
fn base64_url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);

    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;

        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: &str = "http://keyhold.localhost:7717";

    #[test]
    fn tokens_differ_between_runs() {
        assert_ne!(SessionToken::generate().expose(), SessionToken::generate().expose());
    }

    #[test]
    fn token_is_long_enough_to_be_unguessable() {
        // 32 bytes in base64url, unpadded.
        assert_eq!(SessionToken::generate().expose().len(), 43);
    }

    #[test]
    fn only_the_exact_token_matches() {
        let token = SessionToken::generate();
        let value = token.expose().to_string();

        assert!(token.matches(&value));
        assert!(!token.matches(""));
        assert!(!token.matches(&value[..value.len() - 1]));
        assert!(!token.matches(&format!("{value}x")));
    }

    #[test]
    fn a_code_opens_one_session_on_the_origin_that_redeemed_it() {
        let gate = SessionGate::new(7717, false);
        let code = gate.issue_code();

        let Exchange::Granted(session) = gate.exchange(&code, HERE) else { panic!("refused") };

        assert!(gate.check(HERE, &session));
        assert!(!gate.check("http://127.0.0.1:7717", &session));
        assert!(!gate.check(HERE, &code));
    }

    #[test]
    fn a_code_presented_twice_revokes_what_it_opened() {
        let gate = SessionGate::new(7717, false);
        let code = gate.issue_code();

        let Exchange::Granted(session) = gate.exchange(&code, HERE) else { panic!("refused") };

        assert_eq!(gate.exchange(&code, HERE), Exchange::Replayed);
        assert!(!gate.check(HERE, &session));
        // Reported once; after that it is only a wrong code.
        assert_eq!(gate.exchange(&code, HERE), Exchange::Refused);
    }

    #[test]
    fn a_code_expires() {
        let gate = SessionGate::new(7717, false);
        let code = gate.issue_code();
        let later = Instant::now() + CODE_LIFETIME + Duration::from_secs(1);

        assert_eq!(gate.exchange_at(&code, HERE, later), Exchange::Refused);
        assert_eq!(gate.exchange(&code, HERE), Exchange::Refused);
    }

    #[test]
    fn only_the_newest_code_is_outstanding() {
        let gate = SessionGate::new(7717, false);
        let first = gate.issue_code();
        let second = gate.issue_code();

        assert_eq!(gate.exchange(&first, HERE), Exchange::Refused);
        assert!(matches!(gate.exchange(&second, HERE), Exchange::Granted(_)));
    }

    #[test]
    fn guessing_throws_the_outstanding_code_away() {
        let gate = SessionGate::new(7717, false);
        let code = gate.issue_code();

        for _ in 0..WRONG_CODES {
            assert_eq!(gate.exchange("wrong", HERE), Exchange::Refused);
        }

        assert_eq!(gate.exchange(&code, HERE), Exchange::Refused);
    }

    #[test]
    fn renewing_replaces_every_session() {
        let gate = SessionGate::new(7717, false);
        let Exchange::Granted(here) = gate.exchange(&gate.issue_code(), HERE) else { panic!() };
        let Exchange::Granted(there) = gate.exchange(&gate.issue_code(), "http://127.0.0.1:7717") else { panic!() };

        let renewed = gate.renew(HERE);

        assert!(gate.check(HERE, &renewed));
        assert!(!gate.check(HERE, &here));
        assert!(!gate.check("http://127.0.0.1:7717", &there));
    }

    #[test]
    fn the_cookie_is_named_for_the_port_and_hardened_over_https() {
        let plain = SessionGate::new(7717, false);
        let tls = SessionGate::new(7717, true);

        assert_eq!(plain.set_cookie("v"), "keyhold_7717=v; Path=/; HttpOnly; SameSite=Strict");
        assert_eq!(tls.set_cookie("v"), "__Host-keyhold_7717=v; Path=/; HttpOnly; SameSite=Strict; Secure");
        assert_eq!(plain.from_cookie_header("a=1; keyhold_7717=v; keyhold_9000=w"), Some("v"));
        assert_eq!(plain.from_cookie_header("keyhold_9000=w"), None);
    }
}
