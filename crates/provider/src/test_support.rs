//! A throwaway local HTTP server for provider client tests.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

pub(crate) const OK: &str = "HTTP/1.1 200 OK";
pub(crate) const CREATED: &str = "HTTP/1.1 201 Created";
pub(crate) const NOT_FOUND: &str = "HTTP/1.1 404 Not Found";

/// Serves one response per incoming request, in order, then stops. Returns
/// the server's base URL and a handle to `join` once the client under test
/// has made all its calls.
pub(crate) fn serve(responses: &[(&'static str, &'static str)]) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let responses = responses.to_vec();
    let handle = std::thread::spawn(move || {
        for (status_line, body) in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (url, handle)
}

/// [`serve`] for the common single-request case.
pub(crate) fn serve_once(
    status_line: &'static str,
    body: &'static str,
) -> (String, JoinHandle<()>) {
    serve(&[(status_line, body)])
}
