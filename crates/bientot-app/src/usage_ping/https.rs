//! The usage signal over HTTPS with the system TLS (SChannel on Windows): exactly the request of
//! [`PingRequest`], with no cookie, no redirect and no header ureq would add on its own.

use std::error::Error;

use bientot_core::usage_ping::{PingRequest, UsagePingTransport};
use ureq::Agent;
use ureq::http::Request;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub struct HttpsTransport;

impl UsagePingTransport for HttpsTransport {
    fn post(&self, request: &PingRequest) -> Result<u16, Box<dyn Error + Send + Sync>> {
        let response = agent(request).run(http_request(request)?)?;
        Ok(response.status().as_u16())
    }
}

/// An agent that answers every status code, follows no redirect, trusts the system's root
/// certificates and adds no `User-Agent`, `Accept` or `Accept-Encoding` of its own.
fn agent(request: &PingRequest) -> Agent {
    Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .https_only(true)
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent("")
        .accept("")
        .accept_encoding("")
        .timeout_global(Some(request.timeout))
        .build()
        .into()
}

fn http_request(request: &PingRequest) -> Result<Request<&str>, ureq::http::Error> {
    request
        .headers
        .iter()
        .fold(Request::post(request.url), |builder, (name, value)| {
            builder.header(*name, *value)
        })
        .body(request.body.as_str())
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use ureq::config::AutoHeaderValue;
    use ureq::http::Method;

    use super::*;

    fn ping() -> PingRequest {
        PingRequest {
            url: "https://worktime.thanaen.dev/v1/ping",
            headers: [
                ("Content-Type", "application/json"),
                ("User-Agent", "WorkTime"),
            ],
            body: r#"{"version":"1.0.0","channel":"zip","first_this_month":true}"#.to_owned(),
            timeout: Duration::from_secs(10),
        }
    }

    #[test]
    fn the_request_is_a_post_of_the_json_with_two_headers() {
        let ping = ping();
        let request = http_request(&ping).unwrap();

        assert_eq!(request.method(), Method::POST);
        assert_eq!(
            request.uri().to_string(),
            "https://worktime.thanaen.dev/v1/ping"
        );
        let headers: Vec<(&str, &str)> = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().unwrap()))
            .collect();
        assert_eq!(
            headers,
            [
                ("content-type", "application/json"),
                ("user-agent", "WorkTime"),
            ]
        );
        assert_eq!(
            *request.body(),
            r#"{"version":"1.0.0","channel":"zip","first_this_month":true}"#
        );
    }

    /// Against a listener of this test that answers in plain text: the handshake fails in the
    /// system TLS, where a build without it would panic.
    #[test]
    fn the_request_goes_through_the_system_tls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://127.0.0.1:{}/v1/ping",
            listener.local_addr().unwrap().port()
        );
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            // Replying before the ClientHello arrives would close the socket under the client's
            // write, which fails with a broken pipe instead of in the handshake.
            let _ = socket.read(&mut [0; 1024]);
            socket.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").ok();
        });
        let request = PingRequest {
            url: Box::leak(url.into_boxed_str()),
            ..ping()
        };

        let error = HttpsTransport.post(&request).unwrap_err();
        server.join().unwrap();

        assert!(
            matches!(
                error.downcast_ref::<ureq::Error>(),
                Some(ureq::Error::NativeTls(_))
            ),
            "{error:?}"
        );
    }

    #[test]
    fn the_agent_adds_no_header_and_follows_no_redirect() {
        let agent = agent(&ping());
        let config = agent.config();

        for automatic in [
            config.user_agent(),
            config.accept(),
            config.accept_encoding(),
        ] {
            assert!(matches!(automatic, AutoHeaderValue::None), "{automatic:?}");
        }
        assert!(config.https_only());
        assert!(!config.http_status_as_error());
        assert_eq!(config.max_redirects(), 0);
        assert_eq!(config.timeouts().global, Some(Duration::from_secs(10)));
    }
}
