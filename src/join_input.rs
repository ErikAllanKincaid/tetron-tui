//! Join field input: a pasted invite, a file path or a URL
//! (TUI-JOIN-INPUT-001). File and URL bodies are checked by tetron-proto
//! `invite::read_invite`, the rules of `tetron join --invite-file`.
//! tetron-tui has no HTTP client of its own; curl fetches a URL.

use std::path::PathBuf;
use tetron_proto::invite;

/// curl --max-filesize; an invite is at most 72 characters.
const MAX_BYTES: &str = "8192";
/// Whole-transfer limit in seconds.
const MAX_TIME: &str = "15";

/// What the user typed in the join field.
#[derive(Debug, PartialEq, Eq)]
pub enum JoinInput {
    /// A pasted invite.
    Code(String),
    /// A file holding the invite.
    File(PathBuf),
    /// A URL to fetch; `http` is true for plain http://, which needs consent.
    Url { url: String, http: bool },
}

/// Base58 invite codes never contain '/', '.' or '~', so any of them marks
/// a path.
pub fn classify(input: &str) -> JoinInput {
    let t = input.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("https://") {
        JoinInput::Url { url: t.into(), http: false }
    } else if lower.starts_with("http://") {
        JoinInput::Url { url: t.into(), http: true }
    } else if t.starts_with('~') || t.contains(['/', '.']) {
        JoinInput::File(expand_home(t))
    } else {
        JoinInput::Code(t.into())
    }
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(p),
    }
}

/// Read the invite from a file (TUI-JOIN-FILE-001). The file is not changed.
pub fn read_file(path: &std::path::Path) -> Result<String, String> {
    let source = path.display().to_string();
    let f = std::fs::File::open(path).map_err(|e| format!("cannot read invite file {source}: {e}"))?;
    invite::read_invite(f, &source).map_err(|e| e.to_string())
}

/// The fixed curl arguments. No -L: redirects are not followed.
pub fn curl_args(url: &str, allow_http: bool) -> Vec<String> {
    let proto = if allow_http { "=http,https" } else { "=https" };
    ["--proto", proto, "-fsS", "--max-filesize", MAX_BYTES, "--max-time", MAX_TIME, "--", url]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Fetch the invite with curl (TUI-JOIN-URL-001). `allow_http` is the
/// user's consent to a plain http:// fetch. Errors never echo the body.
pub async fn fetch(url: &str, allow_http: bool) -> Result<String, String> {
    let out = tokio::process::Command::new("curl")
        .args(curl_args(url, allow_http))
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "fetch invite: curl is not installed".to_string(),
            _ => format!("fetch invite: can not run curl: {e}"),
        })?;
    if !out.status.success() {
        let code = out.status.code().map_or("signal".to_string(), |c| c.to_string());
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("fetch invite: curl exit {code}: {msg}"));
    }
    invite::read_invite(&out.stdout[..], "fetched invite").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape only: read_invite checks size and lines, not the codec.
    fn code() -> String {
        "3".repeat(72)
    }

    #[test]
    fn classify_code_file_url() {
        let c = code();
        assert_eq!(classify(&format!(" {c} ")), JoinInput::Code(c));
        assert_eq!(classify("https://h/x"), JoinInput::Url { url: "https://h/x".into(), http: false });
        assert_eq!(
            classify("HTTP://10.0.0.5:8000/t"),
            JoinInput::Url { url: "HTTP://10.0.0.5:8000/t".into(), http: true }
        );
        assert_eq!(classify("/etc/x"), JoinInput::File("/etc/x".into()));
        assert_eq!(classify("invite.txt"), JoinInput::File("invite.txt".into()));
        assert_eq!(classify("./a"), JoinInput::File("./a".into()));
    }

    #[test]
    fn tilde_is_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(classify("~/i.txt"), JoinInput::File(PathBuf::from(home).join("i.txt")));
    }

    #[test]
    fn read_file_trims_and_keeps_the_file() {
        let c = code();
        let p = std::env::temp_dir().join(format!("tetron-tui-test-{}.txt", std::process::id()));
        std::fs::write(&p, format!("{c}\n")).unwrap();
        assert_eq!(read_file(&p).unwrap(), c);
        assert!(p.exists());
        std::fs::write(&p, format!("{c}\n{c}\n")).unwrap();
        let e = read_file(&p).unwrap_err();
        assert!(e.contains(&p.display().to_string()) && !e.contains(&c), "{e}");
        std::fs::remove_file(&p).unwrap();
        let e = read_file(&p).unwrap_err();
        assert!(e.contains("cannot read invite file"), "{e}");
    }

    #[test]
    fn curl_args_are_the_fixed_set() {
        assert_eq!(
            curl_args("https://h/x", false).join(" "),
            "--proto =https -fsS --max-filesize 8192 --max-time 15 -- https://h/x"
        );
        assert!(curl_args("http://h/x", true).join(" ").starts_with("--proto =http,https "));
        assert!(!curl_args("https://h/x", false).iter().any(|a| a == "-L"));
    }

    #[tokio::test]
    async fn fetch_refuses_http_without_consent() {
        let e = fetch("http://127.0.0.1:9/x", false).await.unwrap_err();
        assert!(e.contains("curl exit 1"), "{e}");
    }
}
