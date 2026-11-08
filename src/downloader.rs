// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! HTTPS client of the two CDNs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Root of the asset bundle packs.
pub const ABAP_API: &str = "https://resource.pokemon-home.com";
/// Root of the Mitake asset bundles.
pub const MITAKE_API: &str = "https://d2gf339i9nrwgc.cloudfront.net";

/// Tries after the first one when a request fails for a temporary reason.
const RETRIES: u32 = 2;
/// Downloads in a row failing for a temporary reason that stop the download.
pub const MAX_FAILURES_IN_ROW: usize = 20;

/// Client of one CDN root.
pub struct CdnClient {
    api: String,
    agent: ureq::Agent,
    failures_in_row: AtomicUsize,
}

/// Checks whether an error may go away: a network or server error, or too many
/// requests. A missing file will not.
fn is_temporary(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => *code >= 500 || *code == 429,
        _ => true,
    }
}

/// Runs a request, trying again after a temporary error, waiting longer each
/// time.
fn retrying<T>(request: impl Fn() -> Result<T, ureq::Error>) -> Result<T, ureq::Error> {
    for attempt in 0..RETRIES {
        match request() {
            Err(e) if is_temporary(&e) => {
                std::thread::sleep(Duration::from_secs(1 << attempt));
            }
            result => return result,
        }
    }
    request()
}

impl CdnClient {
    /// Creates a client of the CDN root `api`.
    pub fn new(api: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent("Mozilla/5.0 (compatible; MSIE 6.0; Windows 98; Trident/5.1)")
            .timeout_global(Some(Duration::from_secs(100)))
            .build()
            .into();
        Self {
            api: api.to_string(),
            agent,
            failures_in_row: AtomicUsize::new(0),
        }
    }

    /// Returns the URL of `path`, whose unusual characters are percent-encoded.
    fn url(&self, path: &str) -> String {
        let mut url = format!("{}/", self.api);
        for byte in path.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                url.push(char::from(byte));
            } else {
                url.push_str(&format!("%{byte:02X}"));
            }
        }
        url
    }

    /// Checks whether `path` exists on the CDN, which answers 403 or 404 when it
    /// does not. Any other failure is an error, as it tells nothing.
    pub fn exists(&self, path: &str) -> anyhow::Result<bool> {
        let url = self.url(path);
        match retrying(|| self.agent.head(&url).call()) {
            Ok(_) => Ok(true),
            Err(ureq::Error::StatusCode(403 | 404)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Downloads `path` from the CDN root. A failure, an empty response included,
    /// is counted and yields nothing.
    pub fn download_file(&self, path: &str) -> Option<Vec<u8>> {
        let url = self.url(path);
        // read through a configuration, as the default limit is 10 MiB
        let result = retrying(|| {
            self.agent
                .get(&url)
                .call()?
                .body_mut()
                .with_config()
                .read_to_vec()
        });
        self.note_result(result.as_ref().err());
        match result {
            Ok(b) if !b.is_empty() => Some(b),
            _ => {
                crate::ui::fail(&format!("download failed: {path}"));
                None
            }
        }
    }

    /// Counts a temporary failure, any other result ending a series of them.
    fn note_result(&self, error: Option<&ureq::Error>) {
        if error.is_some_and(is_temporary) {
            self.failures_in_row.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failures_in_row.store(0, Ordering::Relaxed);
        }
    }

    /// Checks whether so many downloads in a row failed that going on is useless:
    /// the network or the CDN is probably down.
    pub fn is_down(&self) -> bool {
        self.failures_in_row.load(Ordering::Relaxed) >= MAX_FAILURES_IN_ROW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves one answer of the given status per connection, then stops.
    /// Returns the address, and the thread giving the requests served.
    fn serve(statuses: &[u32]) -> (String, std::thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let statuses = statuses.to_vec();
        let thread = std::thread::spawn(move || {
            for &status in &statuses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 2048];
                let n = stream.read(&mut buffer).unwrap();
                let body = if buffer[..n].starts_with(b"HEAD") {
                    ""
                } else {
                    "data"
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            statuses.len()
        });
        (address, thread)
    }

    #[test]
    fn retries_a_temporary_failure() {
        let (address, thread) = serve(&[500, 503, 200]);
        let cdn = CdnClient::new(&address);
        assert_eq!(cdn.download_file("f").as_deref(), Some(&b"data"[..]));
        assert_eq!(thread.join().unwrap(), 3);
    }

    #[test]
    fn gives_up_at_once_on_a_missing_file() {
        let (address, thread) = serve(&[403]);
        let cdn = CdnClient::new(&address);
        assert_eq!(cdn.download_file("f"), None);
        assert_eq!(thread.join().unwrap(), 1);
    }

    #[test]
    fn reads_more_than_the_default_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let size = 11 * 1024 * 1024;
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.write_all(&vec![7u8; size]).unwrap();
        });
        let data = CdnClient::new(&address).download_file("f").unwrap();
        assert_eq!(data.len(), size);
        thread.join().unwrap();
    }

    #[test]
    fn encodes_unusual_characters() {
        let cdn = CdnClient::new("https://x");
        assert_eq!(cdn.url("a/b-c_d.e"), "https://x/a/b-c_d.e");
        assert_eq!(cdn.url("a b?#é"), "https://x/a%20b%3F%23%C3%A9");
    }

    #[test]
    fn a_series_of_temporary_failures_means_down() {
        let cdn = CdnClient::new("https://x");
        let temporary = ureq::Error::StatusCode(500);
        let missing = ureq::Error::StatusCode(403);
        for _ in 0..MAX_FAILURES_IN_ROW - 1 {
            cdn.note_result(Some(&temporary));
        }
        assert!(!cdn.is_down());

        // a file that is missing shows that the CDN answers
        cdn.note_result(Some(&missing));
        cdn.note_result(Some(&temporary));
        assert!(!cdn.is_down());

        cdn.note_result(None);
        for _ in 0..MAX_FAILURES_IN_ROW {
            cdn.note_result(Some(&temporary));
        }
        assert!(cdn.is_down());
    }

    #[test]
    fn exists_tells_missing_from_unknown() {
        let (address, thread) = serve(&[200, 403, 404, 500, 500, 500]);
        let cdn = CdnClient::new(&address);
        assert!(cdn.exists("f").unwrap());
        assert!(!cdn.exists("f").unwrap());
        assert!(!cdn.exists("f").unwrap());
        assert!(cdn.exists("f").is_err());
        assert_eq!(thread.join().unwrap(), 6);
    }
}
