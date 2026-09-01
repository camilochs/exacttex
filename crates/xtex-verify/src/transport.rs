//! The one seam the network passes through.
//!
//! Production speaks HTTP via `ureq`; every test speaks through a canned
//! transport instead, so the suite runs with zero live network — an
//! unreachable CI is not allowed to decide whether the verifier works.

use std::time::Duration;

/// What a source answered.
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// Body bytes.
    pub body: Vec<u8>,
    /// A `Location` header, when the answer was a redirect.
    pub location: Option<String>,
}

/// Why it did not answer.
#[derive(Debug)]
pub enum TransportError {
    /// The request ran out of time.
    Timeout,
    /// Anything else, said plainly.
    Other(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "timed out"),
            Self::Other(message) => write!(f, "{message}"),
        }
    }
}

/// The seam. `get` follows no redirects: a redirect is an answer.
pub trait Transport {
    /// Fetches `url`, sending `user_agent`, within `timeout`.
    ///
    /// # Errors
    ///
    /// [`TransportError`] when nothing usable came back.
    fn get(
        &self,
        url: &str,
        user_agent: &str,
        timeout: Duration,
    ) -> Result<Response, TransportError>;
}

/// The production transport.
pub struct Http;

impl Transport for Http {
    fn get(
        &self,
        url: &str,
        user_agent: &str,
        timeout: Duration,
    ) -> Result<Response, TransportError> {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(timeout)
            .user_agent(user_agent)
            .build();
        match agent.get(url).call() {
            Ok(response) => {
                use std::io::Read as _;
                let status = response.status();
                let location = response.header("location").map(str::to_owned);
                let mut body = Vec::new();
                let _ = response
                    .into_reader()
                    .take(4 * 1024 * 1024)
                    .read_to_end(&mut body);
                Ok(Response {
                    status,
                    body,
                    location,
                })
            }
            Err(ureq::Error::Status(status, response)) => {
                let location = response.header("location").map(str::to_owned);
                Ok(Response {
                    status,
                    body: Vec::new(),
                    location,
                })
            }
            Err(ureq::Error::Transport(error)) => {
                let text = error.to_string();
                if text.contains("timed out") || text.contains("timeout") {
                    Err(TransportError::Timeout)
                } else {
                    Err(TransportError::Other(text))
                }
            }
        }
    }
}
