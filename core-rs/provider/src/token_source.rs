//! External token sources backing `AuthMethod::TokenCommand`.
//!
//! The Rust kernel does not implement provider-specific OAuth; instead a
//! provider declares a shell command whose stdout is the access token
//! (e.g. `gcloud auth application-default print-access-token` for Vertex AI).
//! This keeps the kernel vendor-agnostic - a different provider plugs in a
//! different command. Platform mode delegates token exchange to the Go control
//! plane and injects the credential instead.
//!
//! The command output is cached for a TTL (token lifetime, ~1h for Google OAuth)
//! so the hot path does not re-run the shell per request.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

/// Error produced when fetching a token from an external source.
#[derive(Debug)]
pub enum TokenError {
    CommandFailed { code: i32, stderr: String },
    EmptyOutput,
    Spawn(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandFailed { code, stderr } => {
                write!(f, "token command exited with {code}: {stderr}")
            }
            Self::EmptyOutput => write!(f, "token command produced no output"),
            Self::Spawn(e) => write!(f, "token command failed to start: {e}"),
        }
    }
}

impl std::error::Error for TokenError {}

/// A source of upstream access tokens (backing `AuthMethod::TokenCommand`).
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Return a currently-valid access token, fetching/refreshing on demand.
    async fn access_token(&self) -> Result<String, TokenError>;
}

/// Cached shell-command token source. Runs `command` through the platform
/// shell (`sh -c` on Unix, `cmd /C` on Windows), takes the first non-empty
/// stdout line as the token, and caches it for `ttl`.
pub struct CommandTokenSource {
    command: String,
    ttl: Duration,
    cache: Mutex<Option<(String, Instant)>>,
}

impl CommandTokenSource {
    pub fn new(command: String, ttl: Duration) -> Self {
        Self {
            command,
            ttl,
            cache: Mutex::new(None),
        }
    }
}

#[async_trait]
impl TokenSource for CommandTokenSource {
    async fn access_token(&self) -> Result<String, TokenError> {
        {
            let cache = self.cache.lock().unwrap();
            if let Some((token, at)) = cache.as_ref() {
                if at.elapsed() < self.ttl {
                    return Ok(token.clone());
                }
            }
        }
        // Cache miss or expired: run the command (at most once per `ttl`).
        let token = run_command(&self.command).await?;
        let mut cache = self.cache.lock().unwrap();
        *cache = Some((token.clone(), Instant::now()));
        Ok(token)
    }
}

/// Run `command` through the platform shell and return the first non-empty
/// stdout line (trimmed). Uses `cmd /C` on Windows (for `gcloud.cmd` shims)
/// and `sh -c` elsewhere.
async fn run_command(command: &str) -> Result<String, TokenError> {
    let cmd = if cfg!(windows) {
        tokio::process::Command::new("cmd")
            .arg("/C")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| TokenError::Spawn(e.to_string()))?
    } else {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| TokenError::Spawn(e.to_string()))?
    };
    let output = cmd
        .wait_with_output()
        .await
        .map_err(|e| TokenError::Spawn(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(TokenError::CommandFailed {
            code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let token = stdout.lines().next().unwrap_or("").trim().to_string();
    if token.is_empty() {
        return Err(TokenError::EmptyOutput);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_command_returns_first_stdout_line() {
        let src = CommandTokenSource::new("echo hello-token".into(), Duration::from_secs(60));
        assert_eq!(src.access_token().await.unwrap(), "hello-token");
    }

    #[tokio::test]
    async fn cached_token_reused_within_ttl() {
        let src = CommandTokenSource::new("echo tok-1".into(), Duration::from_secs(60));
        let a = src.access_token().await.unwrap();
        let b = src.access_token().await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn failed_command_errors() {
        let src = CommandTokenSource::new("exit 1".into(), Duration::from_secs(60));
        assert!(matches!(
            src.access_token().await,
            Err(TokenError::CommandFailed { code, .. }) if code != 0
        ));
    }

    #[tokio::test]
    async fn empty_output_errors() {
        // `exit 0` prints nothing: a command that succeeds but emits no token.
        let src = CommandTokenSource::new("exit 0".into(), Duration::from_secs(60));
        assert!(matches!(src.access_token().await, Err(TokenError::EmptyOutput)));
    }
}
