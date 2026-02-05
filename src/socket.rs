use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum SocketCommand {
    Toggle,
    PushStart,
    PushStop,
    Status,
    Quit,
}

pub struct SocketServer {
    listener: UnixListener,
}

fn socket_path() -> std::path::PathBuf {
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/run/user/{}/tjvox.sock", uid))
}

impl SocketServer {
    pub async fn bind() -> Result<Self> {
        let path = socket_path();

        // Remove stale socket file
        if path.exists() {
            std::fs::remove_file(&path).ok();
        }

        let listener = UnixListener::bind(&path)
            .with_context(|| format!("Failed to bind Unix socket at {:?}", path))?;

        info!("Socket server listening at {:?}", path);
        Ok(Self { listener })
    }

    pub async fn accept(&self) -> Result<(SocketCommand, UnixStream)> {
        let (stream, _addr) = self.listener.accept().await?;

        // Read the command line without consuming the stream
        let line = read_line(&stream).await?;

        let cmd = match line.trim() {
            "toggle" => SocketCommand::Toggle,
            "push-start" => SocketCommand::PushStart,
            "push-stop" => SocketCommand::PushStop,
            "status" => SocketCommand::Status,
            "quit" => SocketCommand::Quit,
            other => {
                warn!("Unknown socket command: {:?}", other);
                return Err(anyhow::anyhow!("Unknown command: {}", other));
            }
        };

        debug!("Received socket command: {:?}", cmd);
        Ok((cmd, stream))
    }

    pub fn cleanup(&self) {
        let path = socket_path();
        std::fs::remove_file(&path).ok();
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Maximum allowed command line length (prevents unbounded memory allocation).
const MAX_LINE_LENGTH: usize = 1024;

async fn read_line(stream: &UnixStream) -> Result<String> {
    let mut buf = Vec::with_capacity(128);
    loop {
        stream.readable().await?;
        let mut tmp = [0u8; 128];
        match stream.try_read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > MAX_LINE_LENGTH {
                    return Err(anyhow::anyhow!(
                        "Command too long ({} bytes, max {})",
                        buf.len(),
                        MAX_LINE_LENGTH
                    ));
                }
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

pub async fn send_command(cmd: &str) -> Result<String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("Failed to connect to daemon socket at {:?}", path))?;

    stream
        .write_all(format!("{}\n", cmd).as_bytes())
        .await?;
    stream.flush().await?;

    // Read response
    let response = read_line(&stream).await?;
    Ok(response.trim().to_string())
}

/// Parse a command string into a SocketCommand (used by tests and accept).
pub fn parse_command(input: &str) -> Result<SocketCommand> {
    match input.trim() {
        "toggle" => Ok(SocketCommand::Toggle),
        "push-start" => Ok(SocketCommand::PushStart),
        "push-stop" => Ok(SocketCommand::PushStop),
        "status" => Ok(SocketCommand::Status),
        "quit" => Ok(SocketCommand::Quit),
        other => Err(anyhow::anyhow!("Unknown command: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_toggle() {
        let cmd = parse_command("toggle").unwrap();
        assert_eq!(cmd, SocketCommand::Toggle);
    }

    #[test]
    fn test_parse_command_push_start() {
        let cmd = parse_command("push-start").unwrap();
        assert_eq!(cmd, SocketCommand::PushStart);
    }

    #[test]
    fn test_parse_command_push_stop() {
        let cmd = parse_command("push-stop").unwrap();
        assert_eq!(cmd, SocketCommand::PushStop);
    }

    #[test]
    fn test_parse_command_status() {
        let cmd = parse_command("status").unwrap();
        assert_eq!(cmd, SocketCommand::Status);
    }

    #[test]
    fn test_parse_command_quit() {
        let cmd = parse_command("quit").unwrap();
        assert_eq!(cmd, SocketCommand::Quit);
    }

    #[test]
    fn test_parse_command_unknown() {
        let result = parse_command("foobar");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_command_with_whitespace() {
        let cmd = parse_command("  toggle  ").unwrap();
        assert_eq!(cmd, SocketCommand::Toggle);
    }

    #[test]
    fn test_parse_command_with_newline() {
        let cmd = parse_command("status\n").unwrap();
        assert_eq!(cmd, SocketCommand::Status);
    }

    #[test]
    fn test_parse_command_empty() {
        let result = parse_command("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_socket_server_bind_and_accept() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sock_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&sock_path).unwrap();
        let server = SocketServer { listener };

        // Spawn a client that sends a command
        let sock_path_clone = sock_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&sock_path_clone).await.unwrap();
            stream.write_all(b"toggle\n").await.unwrap();
            stream.flush().await.unwrap();
        });

        let (cmd, _stream) = server.accept().await.unwrap();
        assert_eq!(cmd, SocketCommand::Toggle);

        client.await.unwrap();
    }

    #[tokio::test]
    async fn test_socket_server_unknown_command() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sock_path = temp_dir.path().join("test.sock");

        let listener = UnixListener::bind(&sock_path).unwrap();
        let server = SocketServer { listener };

        let sock_path_clone = sock_path.clone();
        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&sock_path_clone).await.unwrap();
            stream.write_all(b"invalid-cmd\n").await.unwrap();
            stream.flush().await.unwrap();
        });

        let result = server.accept().await;
        assert!(result.is_err());

        client.await.unwrap();
    }

    #[test]
    fn test_max_line_length_constant() {
        assert_eq!(MAX_LINE_LENGTH, 1024);
    }
}
