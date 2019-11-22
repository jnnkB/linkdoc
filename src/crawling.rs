use crossbeam_channel::{unbounded, Receiver, Sender};
use crossbeam_utils::Backoff;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use url::Url;

use crate::fetching::{fetch_all_urls, url_status, UrlState};

pub struct Crawler {
    active_count: Arc<Mutex<i32>>,
    url_states: Receiver<UrlState>,
}

impl Iterator for Crawler {
    type Item = UrlState;

    fn next(&mut self) -> Option<UrlState> {
        let backoff = Backoff::new();
        loop {
            match self.url_states.try_recv() {
                // If there's currently something in the channel, return
                // it.
                Ok(state) => return Some(state),

                Err(_) => {
                    let active_count_val = self.active_count.lock().unwrap();
                    if *active_count_val == 0 {
                        // We're done, no values left.
                        return None;
                    } else {
                        // The channel is currently empty, but we will
                        // more values later.
                        backoff.snooze();
                        continue;
                    }
                }
            }
        }
    }
}

const THREADS: i32 = 10;

/// Read URLs from the `url_r` channel, and write url states to the
/// `url_states` channel. Write new URLs discovered back to the
/// `url_s` channel.
fn crawl_worker_thread(
    domain: &str,
    url_s: Sender<String>,
    url_r: Receiver<String>,
    visited: Arc<Mutex<HashSet<String>>>,
    active_count: Arc<Mutex<i32>>,
    url_states: Sender<UrlState>,
) {
    loop {
        match url_r.try_recv() {
            Ok(current) => {
                {
                    let mut active_count_val = active_count.lock().unwrap();
                    *active_count_val += 1;
                    assert!(*active_count_val <= THREADS);
                }

                {
                    // Lock `visited` and see if we've already visited this URL.
                    let mut visited_val = visited.lock().unwrap();
                    if visited_val.contains(&current) {
                        // Nothing left to do here, so decrement count.
                        let mut active_count_val = active_count.lock().unwrap();
                        *active_count_val -= 1;
                        continue;
                    } else {
                        visited_val.insert(current.to_owned());
                    }
                }

                // TODO: we are fetching the URL twice, which is silly.
                let state = url_status(&domain, &current);

                // If it's accessible and it's on the same domain:
                if let UrlState::Accessible(ref url) = state.clone() {
                    if url.domain() == Some(&domain) {
                        // then fetch it and append all the URLs found.
                        for new_url in fetch_all_urls(&url) {
                            url_s.send(new_url).unwrap();
                        }
                    }
                }

                {
                    // This thread is now done, so decrement the count.
                    let mut active_count_val = active_count.lock().unwrap();
                    *active_count_val -= 1;
                    assert!(*active_count_val >= 0);
                }

                url_states.send(state).unwrap();
            }
            Err(_) => {
                let active_count_val = active_count.lock().unwrap();
                // Nothing in the channel for us to do.
                // If there are requests still in flight, we might
                // get more work in the future.
                if *active_count_val > 0 {
                    // snooze
                } else {
                    // There won't be any more URLs to visit, so terminate this thread.
                    break;
                }
            }
        }
    }
}

/// Starting at start_url, recursively iterate over all the URLs which match
/// the domain, and return an iterator of their URL status.
pub fn crawl(domain: &str, start_url: &Url) -> Crawler {
    let active_count = Arc::new(Mutex::new(0));
    let visited = Arc::new(Mutex::new(HashSet::new()));

    let (url_state_s, url_state_r) = unbounded();
    let (visit_s, visit_r) = unbounded();
    visit_s.send(start_url.as_str().into()).unwrap();

    let crawler = Crawler {
        active_count: active_count.clone(),
        url_states: url_state_r,
    };

    for _ in 0..THREADS {
        let domain = domain.to_owned();
        let visited = visited.clone();
        let active_count = active_count.clone();
        let url_state_s = url_state_s.clone();
        let visit_r = visit_r.clone();
        let visit_s = visit_s.clone();

        thread::spawn(move || {
            crawl_worker_thread(
                &domain,
                visit_s,
                visit_r,
                visited,
                active_count,
                url_state_s,
            );
        });
    }

    crawler
}
