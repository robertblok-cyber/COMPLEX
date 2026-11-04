// RobertBlok.com — Fullstack development
//
// A small HTTP/1.1 server on the Rust standard library only.
// Every limit is deliberate: a header cap and a read timeout keep one slow
// or hostile client from holding the server hostage (Slowloris).
// TLS is not done here on purpose: put Caddy or Nginx in front for HTTPS.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub extra_headers: Vec<(&'static str, String)>,
}

impl Response {
    pub fn html(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Response { status, content_type: "text/html; charset=utf-8", body: body.into(), extra_headers: Vec::new() }
    }

    pub fn file(content_type: &'static str, body: Vec<u8>) -> Self {
        Response { status: 200, content_type, body, extra_headers: vec![("Cache-Control", "public, max-age=3600".into())] }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Response { status, content_type: "text/plain; charset=utf-8", body: body.as_bytes().to_vec(), extra_headers: Vec::new() }
    }

    fn write_to(&self, w: &mut impl Write, head_only: bool) -> io::Result<()> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            431 => "Request Header Fields Too Large",
            500 => "Internal Server Error",
            _ => "Unknown",
        };
        write!(
            w,
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: strict-origin-when-cross-origin\r\nServer: complex/0.1\r\n",
            self.status,
            reason,
            self.content_type,
            self.body.len()
        )?;
        for (k, v) in &self.extra_headers {
            write!(w, "{k}: {v}\r\n")?;
        }
        w.write_all(b"\r\n")?;
        if !head_only {
            w.write_all(&self.body)?;
        }
        w.flush()
    }
}

pub type Handler = dyn Fn(&Request) -> Response + Send + Sync;

/// Listens forever; each connection is served on its own thread.
pub fn serve(addr: &str, handler: Arc<Handler>, log: bool) -> io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let handler = Arc::clone(&handler);
        thread::spawn(move || {
            let _ = handle_connection(stream, &*handler, log);
        });
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, handler: &Handler, log: bool) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let peer = stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();

    let (response, line) = match read_request(&mut stream) {
        Ok(req) => {
            let res = handler(&req);
            let head_only = req.method == "HEAD";
            let line = format!("{} {} {} {}", peer, req.method, req.path, res.status);
            res.write_to(&mut stream, head_only)?;
            (res.status, line)
        }
        Err(ParseError::TooLarge) => {
            Response::text(431, "request header too large").write_to(&mut stream, false)?;
            (431, format!("{peer} - - 431"))
        }
        Err(ParseError::BodyTooLarge) => {
            Response::text(413, "request body too large").write_to(&mut stream, false)?;
            (413, format!("{peer} - - 413"))
        }
        Err(ParseError::Malformed) => {
            Response::text(400, "bad request").write_to(&mut stream, false)?;
            (400, format!("{peer} - - 400"))
        }
        Err(ParseError::Io(e)) => return Err(e),
    };
    if log {
        eprintln!("{} {line}", crate::interp::format_utc(crate::interp::unix_now()));
    }
    let _ = response;
    Ok(())
}

enum ParseError {
    TooLarge,
    BodyTooLarge,
    Malformed,
    Io(io::Error),
}

impl From<io::Error> for ParseError {
    fn from(e: io::Error) -> Self {
        ParseError::Io(e)
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, ParseError> {
    let mut reader = BufReader::new(stream);
    let mut total = 0usize;

    let mut line = String::new();
    reader.read_line(&mut line)?;
    total += line.len();
    if total > MAX_HEADER_BYTES {
        return Err(ParseError::TooLarge);
    }
    let mut parts = line.trim_end().split(' ');
    let method = parts.next().ok_or(ParseError::Malformed)?.to_string();
    let target = parts.next().ok_or(ParseError::Malformed)?;
    let version = parts.next().ok_or(ParseError::Malformed)?;
    if !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return Err(ParseError::Malformed);
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };

    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        total += h.len();
        if total > MAX_HEADER_BYTES {
            return Err(ParseError::TooLarge);
        }
        let h = h.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':').ok_or(ParseError::Malformed)?;
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }

    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY_BYTES {
        return Err(ParseError::BodyTooLarge);
    }
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok(Request { method, path, query, headers, body })
}
