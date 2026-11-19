// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What a failed call to another service (the review service, Gitea, Discord) says in a log line or an answer: its kind or its
//! status, never its address. The text of ureq 2's error is the whole request URL plus the reason; a Discord webhook's URL is
//! its token, and the review service's and Gitea's are internal addresses a builder has no need to see.

/// Why a call failed, without its URL: "it answered 503", "Connection Failed", "Dns Failed"...
pub fn why(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("it answered {code}"),
        ureq::Error::Transport(t) => t.kind().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_call_says_why_without_its_address() {
        // Nothing listens on port 1: the call fails as a webhook does when Discord can't be reached.
        let e = ureq::get("http://127.0.0.1:1/api/webhooks/123/SECRETTOKEN").timeout(std::time::Duration::from_secs(5)).call().unwrap_err();
        assert!(e.to_string().contains("SECRETTOKEN"), "ureq's own text has the address: {e}");
        let said = why(&e);
        assert!(!said.contains("SECRETTOKEN") && !said.contains("127.0.0.1") && !said.is_empty(), "{said}");
    }
}
