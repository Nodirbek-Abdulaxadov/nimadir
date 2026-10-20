//! The network, for guests — `fetch` and `fetch-poll` in the WIT, ADR-3.
//!
//! Every request is checked on the way out: over TLS, to an origin the app's
//! manifest declared, which the user has allowed. Requests run on threads and
//! are polled from frames, so a slow server never holds a frame.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Requests one app may have under way at once.
pub const MAX_IN_FLIGHT: usize = 8;
/// The largest body, sent or received: 8 MiB.
pub const MAX_BODY: usize = 8 << 20;

pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub type Answer = Result<Response, String>;

enum State {
    /// Held until the user answers whether `origin` may be reached.
    Waiting {
        origin: String,
        request: Request,
    },
    Flying(Receiver<Answer>),
    Done(Answer),
}

/// One app's network: what it declared, what the user allowed, what is under
/// way.
pub struct Net {
    /// Origins the manifest lists, normalised (`https://host[:port]`).
    declared: Vec<String>,
    /// Where a signed app's grants are remembered; `None` for an unsigned
    /// one, which is asked each run.
    grants_file: Option<PathBuf>,
    granted: HashSet<String>,
    /// Refused this run. Not remembered: the user may answer differently
    /// next time.
    denied: HashSet<String>,
    /// The origin the user is being asked about, if any.
    asking: Option<String>,
    next: u32,
    requests: HashMap<u32, State>,
}

impl Net {
    /// `declared` is what the manifest lists under `network`; `identity` is
    /// the signed app's, whose grants are kept.
    pub fn new(declared: &[String], identity: Option<&str>) -> Self {
        let grants_file = identity.and_then(|id| {
            crate::paths::data_dir().map(|d| d.join("apps").join(id).join("grants.json"))
        });
        let granted = grants_file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .collect();
        Net {
            declared: declared.iter().filter_map(|o| origin_of(o).ok()).collect(),
            grants_file,
            granted,
            denied: HashSet::new(),
            asking: None,
            next: 0,
            requests: HashMap::new(),
        }
    }

    pub fn fetch(&mut self, request: Request) -> Result<u32, String> {
        crate::resolve::require_tls(&request.url).map_err(|e| e.to_string())?;
        let origin = origin_of(&request.url)?;
        if self.declared.is_empty() {
            return Err("this app declares no network access in its manifest".into());
        }
        if !self.declared.contains(&origin) {
            return Err(format!(
                "{origin} is not among the origins this app's manifest declares"
            ));
        }
        if self.denied.contains(&origin) {
            return Err(format!("the user did not allow connecting to {origin}"));
        }
        if request.body.len() > MAX_BODY {
            return Err(format!("a request body may be {} MiB", MAX_BODY >> 20));
        }
        let open = self
            .requests
            .values()
            .filter(|s| !matches!(s, State::Done(_)))
            .count();
        if open >= MAX_IN_FLIGHT {
            return Err(format!("{MAX_IN_FLIGHT} requests are already under way"));
        }

        let id = self.next;
        self.next = self.next.wrapping_add(1);
        let state = if self.granted.contains(&origin) {
            State::Flying(spawn(request))
        } else {
            if self.asking.is_none() {
                self.asking = Some(origin.clone());
            }
            State::Waiting { origin, request }
        };
        self.requests.insert(id, state);
        Ok(id)
    }

    /// `None` while under way; the answer once, after which the number is
    /// forgotten.
    pub fn poll(&mut self, id: u32) -> Option<Answer> {
        let Some(state) = self.requests.get_mut(&id) else {
            return Some(Err(format!("no request numbered {id}")));
        };
        if let State::Flying(rx) = state {
            match rx.try_recv() {
                Ok(answer) => *state = State::Done(answer),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    *state = State::Done(Err("the request ended without an answer".into()))
                }
            }
        }
        match self.requests.remove(&id) {
            Some(State::Done(answer)) => Some(answer),
            Some(other) => {
                self.requests.insert(id, other);
                None
            }
            None => None,
        }
    }

    /// The origin the user is being asked about.
    pub fn asking(&self) -> Option<&str> {
        self.asking.as_deref()
    }

    /// The user's answer about `asking()`. Requests held for it go, or fail;
    /// the next origin waiting, if any, is asked about next.
    pub fn answer(&mut self, allow: bool) {
        let Some(origin) = self.asking.take() else {
            return;
        };
        if allow {
            self.granted.insert(origin.clone());
            self.remember();
        } else {
            self.denied.insert(origin.clone());
        }
        crate::diag::line(&format!(
            "network to {origin} {}",
            if allow { "allowed" } else { "not allowed" }
        ));
        for state in self.requests.values_mut() {
            if matches!(state, State::Waiting { origin: o, .. } if *o == origin) {
                let State::Waiting { request, .. } =
                    std::mem::replace(state, State::Done(Err(String::new())))
                else {
                    unreachable!("matched as waiting just above");
                };
                *state = if allow {
                    State::Flying(spawn(request))
                } else {
                    State::Done(Err(format!(
                        "the user did not allow connecting to {origin}"
                    )))
                };
            }
        }
        self.asking = self.requests.values().find_map(|s| match s {
            State::Waiting { origin, .. } => Some(origin.clone()),
            _ => None,
        });
    }

    /// Whether a request is out on the network. One waiting on the user is
    /// not: their answer arrives as input, which wakes the window anyway.
    pub fn flying(&self) -> bool {
        self.requests
            .values()
            .any(|s| matches!(s, State::Flying(_)))
    }

    /// Block until every request under way has its answer, or `timeout` has
    /// passed. For the headless runner, which has no frames to wait through.
    pub fn settle(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        for state in self.requests.values_mut() {
            if let State::Flying(rx) = state {
                let left = deadline.saturating_duration_since(Instant::now());
                *state = State::Done(
                    rx.recv_timeout(left)
                        .unwrap_or_else(|_| Err("no answer in time".into())),
                );
            }
        }
    }

    fn remember(&self) {
        let Some(file) = &self.grants_file else {
            return;
        };
        let mut origins: Vec<&String> = self.granted.iter().collect();
        origins.sort();
        if let (Some(dir), Ok(json)) = (file.parent(), serde_json::to_vec(&origins)) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(file, json);
        }
    }
}

/// `scheme://host[:port]`, lower-case, the default port left off — so a
/// manifest's `https://API.example.com:443/` and a request to
/// `https://api.example.com/v1` are the same origin.
pub fn origin_of(url: &str) -> Result<String, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("{url} is not an http or https address"))?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => return Err(format!("{url} is not an http or https address")),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, h)| h)
        .to_ascii_lowercase();
    let host = host.strip_suffix(default_port).unwrap_or(&host);
    if host.is_empty() {
        return Err(format!("{url} has no host"));
    }
    Ok(format!("{scheme}://{host}"))
}

fn spawn(request: Request) -> Receiver<Answer> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(perform(request));
    });
    rx
}

fn perform(r: Request) -> Answer {
    // No redirects followed: one would take the request to an origin nobody
    // checked. A 3xx comes back to the app, whose next `fetch` is checked.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .redirects(0)
        .build();
    let mut req = agent.request(&r.method, &r.url);
    for (name, value) in &r.headers {
        req = req.set(name, value);
    }
    let sent = if r.body.is_empty() {
        req.call()
    } else {
        req.send_bytes(&r.body)
    };
    let resp = match sent {
        Ok(resp) => resp,
        // A status is an answer, whatever it is.
        Err(ureq::Error::Status(_, resp)) => resp,
        Err(e) => return Err(e.to_string()),
    };
    let status = resp.status();
    let headers = resp
        .headers_names()
        .into_iter()
        .filter_map(|name| resp.header(&name).map(|v| (name.clone(), v.to_string())))
        .collect();
    let mut body = Vec::new();
    resp.into_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("reading the response: {e}"))?;
    if body.len() > MAX_BODY {
        return Err(format!(
            "the response is larger than {} MiB",
            MAX_BODY >> 20
        ));
    }
    Ok(Response {
        status,
        headers,
        body,
    })
}
