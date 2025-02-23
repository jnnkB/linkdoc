use colored::*;
use crossbeam_channel::{select, unbounded};
use reqwest::StatusCode;
use std::fmt;
use std::thread;
use std::time::Duration;
use url::{ParseError, Url};

use crate::parsing;

#[derive(Debug, Clone)]
pub enum UrlState {
    Accessible(String, Url),
    BadStatus(String, Url, StatusCode),
    ConnectionFailed(String, Url),
    TimedOut(String, Url),
    Malformed(String, String),
}

impl fmt::Display for UrlState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let tick = "✔".green();
        let cross = "✘".red();
        match *self {
            UrlState::Accessible(ref old_url, ref url) => {
                format!("{} {} {}", tick, old_url, url).fmt(f)
            }
            UrlState::BadStatus(ref old_url, ref url, ref status) => {
                format!("{} {} {} ({})", cross, old_url, url, status).fmt(f)
            }
            UrlState::ConnectionFailed(ref old_url, ref url) => {
                format!("{} {} {} (connection failed)", cross, old_url, url).fmt(f)
            }
            UrlState::TimedOut(ref old_url, ref url) => {
                format!("{} {} {} (timed out)", cross, old_url, url).fmt(f)
            }
            UrlState::Malformed(ref old_url, ref url) => {
                format!("{} {} {} (malformed)", cross, old_url, url).fmt(f)
            }
        }
    }
}

fn build_url(domain: &str, path: &str) -> Result<Url, ParseError> {
    let base_url_string = format!("http://{}", domain);
    let base_url = Url::parse(&base_url_string)?;
    base_url.join(path)
}

const TIMEOUT_SECS: u64 = 10;

pub fn url_status(domain: &str, old_path: &str, path: &str) -> UrlState {
    match build_url(domain, path) {
        Ok(url) => {
            let (s, r) = unbounded();
            let url2 = url.clone();
            let old_path_static = old_path.to_owned();

            // Try to do the request.
            thread::spawn(move || {
                let response = reqwest::get(url.as_str());

                let _ = s.send(match response {
                    Ok(response) => {
                        if response.status().is_success() {
                            UrlState::Accessible(old_path_static, url)
                        } else {
                            // TODO: allow redirects unless they're circular
                            UrlState::BadStatus(old_path_static, url, response.status())
                        }
                    }
                    Err(_) => UrlState::ConnectionFailed(old_path_static, url),
                });
            });

            // Return the request result, or timeout.
            select! {
                recv(r) -> msg => msg.unwrap(),
                default(Duration::from_secs(TIMEOUT_SECS)) => UrlState::TimedOut(old_path.to_owned(), url2)
            }
        }
        Err(_) => UrlState::Malformed(old_path.to_owned(), path.to_owned()),
    }
}

pub fn fetch_url(url: &Url) -> String {
    // Creating an outgoing request.
    let mut res = reqwest::get(url.as_str()).expect("could not fetch URL");

    // Read the body.
    match res.text() {
        Ok(body) => body,
        // TODO: handle malformed data more gracefully.
        Err(_) => String::new(),
    }
}

/// Fetch the requested URL, and return a list of all the URLs on the
/// page. We deliberately return strings because we're also interested
/// in malformed URLs.
pub fn fetch_all_urls(url: &Url) -> Vec<String> {
    let html_src = fetch_url(url);
    parsing::get_urls(&html_src)
}
