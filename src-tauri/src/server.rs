use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::error::AuthError;

/// The registered redirect URI is exactly http://127.0.0.1:8917/callback —
/// a different port would not match it, so we never fall back to another one.
const BIND_ADDR: &str = "127.0.0.1:8917";
const CALLBACK_PATH: &str = "/callback";
const CALLBACK_DEADLINE: Duration = Duration::from_secs(300);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STREAM_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Query parameters Spotify appends to the loopback redirect.
#[derive(Debug, PartialEq, Eq)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Parse the request target of the callback request
/// (e.g. `/callback?code=AQ..&state=xyz`) into its query parameters.
/// Anything that is not `/callback` is rejected.
pub fn parse_callback_target(target: &str) -> Result<CallbackParams, AuthError> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| AuthError::BadCallback)?;
    if url.path() != CALLBACK_PATH {
        return Err(AuthError::BadCallback);
    }
    let mut params = CallbackParams {
        code: None,
        state: None,
        error: None,
    };
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => params.code = Some(value.into_owned()),
            "state" => params.state = Some(value.into_owned()),
            "error" => params.error = Some(value.into_owned()),
            _ => {}
        }
    }
    Ok(params)
}

/// Bind the loopback listener. Done before opening the browser so a busy port
/// fails fast with a clear message instead of a dead redirect.
pub fn bind_listener() -> Result<TcpListener, AuthError> {
    let listener = TcpListener::bind(BIND_ADDR).map_err(|e| {
        if e.kind() == ErrorKind::AddrInUse {
            AuthError::PortInUse
        } else {
            AuthError::Config(format!("cannot bind loopback listener: {e}"))
        }
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|e| AuthError::Config(format!("cannot configure listener: {e}")))?;
    Ok(listener)
}

/// Block until the OAuth callback arrives (or the 5-minute deadline passes),
/// then drop the listener. Stray requests (favicon etc.) get a 404 and the
/// wait continues. Runs on a blocking thread, never on the async runtime.
pub fn wait_for_callback(listener: TcpListener) -> Result<CallbackParams, AuthError> {
    let deadline = Instant::now() + CALLBACK_DEADLINE;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _addr)) => {
                if let Some(params) = handle_connection(stream) {
                    return Ok(params);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(_) => std::thread::sleep(ACCEPT_POLL_INTERVAL),
        }
    }
    Err(AuthError::CallbackTimeout)
}

/// Read one HTTP request; answer it; return the params if it was the callback.
fn handle_connection(stream: TcpStream) -> Option<CallbackParams> {
    // I/O errors on a single connection are non-fatal: answer what we can and
    // keep waiting for the real callback.
    if stream.set_read_timeout(Some(STREAM_IO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(STREAM_IO_TIMEOUT)).is_err()
        || stream.set_nonblocking(false).is_err()
    {
        return None;
    }
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return None;
    }
    let mut stream = reader.into_inner();
    let target = match parse_request_line(&request_line) {
        Some(t) => t,
        None => {
            respond(&mut stream, "400 Bad Request", "Bad request.");
            return None;
        }
    };
    match parse_callback_target(&target) {
        Ok(params) => {
            respond(
                &mut stream,
                "200 OK",
                "<strong>Cued</strong> received the Spotify response. You can close this tab.",
            );
            Some(params)
        }
        Err(_) => {
            respond(&mut stream, "404 Not Found", "Not found.");
            None
        }
    }
}

fn parse_request_line(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    if method != "GET" {
        return None;
    }
    Some(target.to_owned())
}

fn respond(stream: &mut TcpStream, status: &str, body_text: &str) {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Cued</title></head>\
         <body style=\"font-family:system-ui;background:#0C0F0D;color:#EAF0EC;\
         display:flex;justify-content:center;margin-top:20vh\"><p>{body_text}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // Best effort: the browser closing early must not kill the login flow.
    let _write_result = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_and_state() {
        let p = parse_callback_target("/callback?code=AQDx1o&state=xyz-_9").expect("must parse");
        assert_eq!(p.code.as_deref(), Some("AQDx1o"));
        assert_eq!(p.state.as_deref(), Some("xyz-_9"));
        assert!(p.error.is_none());
    }

    #[test]
    fn parses_error_param_on_denial() {
        let p =
            parse_callback_target("/callback?error=access_denied&state=xyz").expect("must parse");
        assert_eq!(p.error.as_deref(), Some("access_denied"));
        assert!(p.code.is_none());
    }

    #[test]
    fn percent_decodes_values() {
        let p = parse_callback_target("/callback?code=a%2Bb&state=s").expect("must parse");
        assert_eq!(p.code.as_deref(), Some("a+b"));
    }

    #[test]
    fn rejects_other_paths() {
        assert!(parse_callback_target("/favicon.ico").is_err());
        assert!(parse_callback_target("/").is_err());
    }
}
