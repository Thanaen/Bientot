//! The request for the revocation list (`specs/003-plugin-runtime/contracts/revocations.md`):
//! a plain `GET` with no query string, no cookie and no identifier, over the system TLS.

/// Whether this build asks for the list: only a release build that may reach the network. A
/// debug build, and one without the network, goes by the last list it kept.
pub const FETCHES: bool = cfg!(all(feature = "usage-ping", not(debug_assertions)));

/// The list's bytes, or why none came. Blocks: call it off the main thread.
#[cfg(feature = "usage-ping")]
pub fn fetch() -> Result<Vec<u8>, String> {
    use std::time::Duration;

    use bientot_plugins::revocation::{MAX_BYTES, URL};
    use ureq::Agent;

    use crate::platform;

    let agent: Agent = Agent::config_builder()
        .tls_config(platform::system_tls())
        .https_only(true)
        .max_redirects(0)
        // The usage signal's, the only other request Bientôt makes on its own.
        .user_agent("WorkTime")
        .accept("")
        .accept_encoding("")
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut response = agent.get(URL).call().map_err(|error| error.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES as u64 + 1)
        .read_to_vec()
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "usage-ping"))]
pub fn fetch() -> Result<Vec<u8>, String> {
    Err("this build does not fetch the list".to_owned())
}
