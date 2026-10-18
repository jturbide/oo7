use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    time::timeout,
};
use zeroize::Zeroizing;

use crate::protocol::{PamMessage, PamResponse};

/// Deadline for the complete socket exchange (in milliseconds).
const SOCKET_TIMEOUT_MS: u64 = 5000;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

struct SavedSignals {
    sigpipe: libc::sigaction,
    sigchld: libc::sigaction,
}

impl SavedSignals {
    fn new() -> Self {
        unsafe {
            let mut saved = Self {
                sigpipe: std::mem::zeroed(),
                sigchld: std::mem::zeroed(),
            };

            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = libc::SIG_IGN;
            libc::sigaction(libc::SIGPIPE, &action, &mut saved.sigpipe);

            action.sa_sigaction = libc::SIG_DFL;
            libc::sigaction(libc::SIGCHLD, &action, &mut saved.sigchld);

            saved
        }
    }
}

impl Drop for SavedSignals {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGPIPE, &self.sigpipe, std::ptr::null_mut());
            libc::sigaction(libc::SIGCHLD, &self.sigchld, std::ptr::null_mut());
        }
    }
}

/// Error type for socket operations
#[derive(Debug)]
pub enum SocketError {
    Connect(io::Error),
    Send(io::Error),
    Receive(io::Error),
    Serialize(zgvariant::Error),
    Timeout,
    InvalidSocket(String),
    InvalidResponse(&'static str),
    Rejected,
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "Failed to connect to daemon socket: {e}"),
            Self::Send(e) => write!(f, "Failed to send message: {e}"),
            Self::Receive(e) => write!(f, "Failed to receive daemon response: {e}"),
            Self::Serialize(e) => write!(f, "Failed to serialize message: {e}"),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::InvalidSocket(msg) => write!(f, "Invalid socket: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "Invalid daemon response: {msg}"),
            Self::Rejected => write!(f, "Daemon rejected request"),
        }
    }
}

impl std::error::Error for SocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(e) | Self::Send(e) | Self::Receive(e) => Some(e),
            Self::Serialize(e) => Some(e),
            Self::Timeout | Self::InvalidSocket(_) | Self::InvalidResponse(_) | Self::Rejected => {
                None
            }
        }
    }
}

pub fn send_secret_to_daemon(
    message: PamMessage,
    uid: u32,
    gid: u32,
    auto_start: bool,
) -> Result<(), SocketError> {
    // Check if we're already running as the target user
    let current_uid = unsafe { libc::getuid() };
    let current_gid = unsafe { libc::getgid() };
    let current_euid = unsafe { libc::geteuid() };
    let current_egid = unsafe { libc::getegid() };

    if uid == current_uid && gid == current_gid && uid == current_euid && gid == current_egid {
        tracing::debug!("Already running as target user (UID={uid}, GID={gid})",);
        let runtime = tokio::runtime::Runtime::new().map_err(SocketError::Connect)?;
        return runtime
            .block_on(async { send_secret_to_daemon_async(message, uid, auto_start, None).await });
    }

    // Need to fork and switch credentials
    tracing::debug!(
        "Running as different user (current UID={current_uid}, target UID={uid}), forking to switch credentials"
    );

    let saved_signals = SavedSignals::new();

    match unsafe { libc::fork() } {
        -1 => {
            drop(saved_signals);
            tracing::error!("Failed to fork process for credential switch");
            Err(SocketError::Connect(io::Error::last_os_error()))
        }
        0 => {
            unsafe {
                // Do not pass the privileged caller's supplementary groups to
                // the target user or the login helper it may
                // exec.
                if (current_euid == 0 && libc::setgroups(0, std::ptr::null()) < 0)
                    || libc::setgid(gid) < 0
                    || libc::setuid(uid) < 0
                    || libc::setegid(gid) < 0
                    || libc::seteuid(uid) < 0
                {
                    tracing::error!(
                        "Failed to switch to user credentials (UID={uid}, GID={gid}): {}",
                        io::Error::last_os_error()
                    );
                    libc::_exit(1);
                }
            }

            tracing::debug!("Child process switched to UID={uid}, GID={gid}");

            let runtime = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("Failed to create tokio runtime in child: {e}",);
                    unsafe { libc::_exit(1) };
                }
            };

            let result = runtime.block_on(async {
                send_secret_to_daemon_async(message, uid, auto_start, None).await
            });

            match result {
                Ok(_) => unsafe { libc::_exit(0) },
                Err(e) => {
                    tracing::error!("Failed to send message in child process: {e}",);
                    unsafe { libc::_exit(1) }
                }
            }
        }
        child_pid => {
            // Parent process - wait for child to complete
            tracing::debug!("Forked child process with PID {child_pid}",);
            let mut status: libc::c_int = 0;

            loop {
                let wait_result = unsafe { libc::waitpid(child_pid, &mut status, 0) };
                if wait_result == child_pid {
                    break;
                } else if wait_result == -1 {
                    let err = io::Error::last_os_error();
                    if err.kind() != io::ErrorKind::Interrupted {
                        drop(saved_signals);
                        tracing::error!("Failed to wait for child process: {err}",);
                        return Err(SocketError::Connect(err));
                    }
                }
            }

            drop(saved_signals);

            if libc::WIFEXITED(status) {
                let exit_code = libc::WEXITSTATUS(status);
                if exit_code == 0 {
                    tracing::debug!("Child process completed successfully");
                    Ok(())
                } else {
                    tracing::error!("Child process exited with code {exit_code}");
                    Err(SocketError::Connect(io::Error::other(format!(
                        "Child process failed with exit code {exit_code}"
                    ))))
                }
            } else {
                tracing::error!("Child process terminated abnormally");
                Err(SocketError::Connect(io::Error::other(
                    "Child process terminated abnormally",
                )))
            }
        }
    }
}

/// Start the oo7-daemon-login helper, passing the secret via stdin pipe.
///
/// The helper holds the secret on a socket until oo7-daemon connects and
/// retrieves it via memfd.
fn start_login_helper(secret: &[u8]) -> Result<(), SocketError> {
    tracing::info!("Starting oo7-daemon-login helper");

    let mut pipe_fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } < 0 {
        return Err(SocketError::Connect(io::Error::last_os_error()));
    }
    let pipe_read = pipe_fds[0];
    let pipe_write = pipe_fds[1];

    let saved_signals = SavedSignals::new();

    match unsafe { libc::fork() } {
        -1 => {
            drop(saved_signals);
            unsafe {
                libc::close(pipe_read);
                libc::close(pipe_write);
            }
            Err(SocketError::Connect(io::Error::last_os_error()))
        }
        0 => unsafe {
            libc::close(pipe_write);

            libc::dup2(pipe_read, 0);
            if pipe_read > 0 {
                libc::close(pipe_read);
            }

            let path = std::ffi::CString::new(
                option_env!("OO7_DAEMON_LOGIN_PATH").unwrap_or("/usr/libexec/oo7-daemon-login"),
            )
            .unwrap();
            let args = [path.as_ptr(), std::ptr::null()];
            libc::execv(path.as_ptr(), args.as_ptr());

            libc::_exit(1);
        },
        child_pid => {
            unsafe {
                libc::close(pipe_read);

                let mut written = 0;
                while written < secret.len() {
                    let n = libc::write(
                        pipe_write,
                        secret.as_ptr().add(written) as *const libc::c_void,
                        secret.len() - written,
                    );
                    if n <= 0 {
                        break;
                    }
                    written += n as usize;
                }
                libc::close(pipe_write);
            }

            drop(saved_signals);
            tracing::info!("Started oo7-daemon-login with PID {child_pid}");
            Ok(())
        }
    }
}

async fn send_secret_to_daemon_async(
    message: PamMessage,
    uid: u32,
    auto_start: bool,
    socket_path: Option<PathBuf>,
) -> Result<(), SocketError> {
    timeout(
        Duration::from_millis(SOCKET_TIMEOUT_MS),
        exchange_with_daemon(message, uid, auto_start, socket_path),
    )
    .await
    .map_err(|_| SocketError::Timeout)?
}

async fn exchange_with_daemon(
    message: PamMessage,
    uid: u32,
    auto_start: bool,
    socket_path: Option<PathBuf>,
) -> Result<(), SocketError> {
    let socket_path = socket_path
        .or_else(|| std::env::var("OO7_PAM_SOCKET").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}/oo7-pam.sock")));

    tracing::debug!("Connecting to daemon socket at: {}", socket_path.display());

    // Validate socket ownership before connecting to prevent interception
    // by a fake socket placed by another user.
    match socket_path.symlink_metadata() {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(SocketError::InvalidSocket(
                    "socket path is a symlink".to_string(),
                ));
            }
            if !meta.file_type().is_socket() {
                return Err(SocketError::InvalidSocket(
                    "path is not a socket".to_string(),
                ));
            }
            if meta.uid() != uid {
                return Err(SocketError::InvalidSocket(format!(
                    "socket owned by UID {} but expected {}",
                    meta.uid(),
                    uid
                )));
            }
        }
        Err(e) if auto_start && e.kind() == io::ErrorKind::NotFound => {
            tracing::info!("Socket not found, starting login helper");
            start_login_helper(&message.new_secret)?;
            return Ok(());
        }
        Err(e) => {
            return Err(SocketError::Connect(e));
        }
    }

    let mut stream = match UnixStream::connect(&socket_path).await {
        Ok(s) => s,
        Err(e) if auto_start => {
            tracing::info!("Daemon not running ({}), starting login helper", e.kind());
            start_login_helper(&message.new_secret)?;
            return Ok(());
        }
        Err(e) => {
            return Err(SocketError::Connect(e));
        }
    };

    tracing::debug!("Connected to daemon socket");

    tracing::debug!("Sending message for user {}", message.username);
    let message_bytes = Zeroizing::new(message.to_bytes().map_err(SocketError::Serialize)?);

    let length = message_bytes.len() as u32;
    stream
        .write_all(&length.to_le_bytes())
        .await
        .map_err(SocketError::Send)?;

    stream
        .write_all(&message_bytes)
        .await
        .map_err(SocketError::Send)?;

    stream.flush().await.map_err(SocketError::Send)?;

    tracing::debug!("Sent message to daemon, reading response");

    let mut length_bytes = [0u8; 4];
    stream
        .read_exact(&mut length_bytes)
        .await
        .map_err(SocketError::Receive)?;
    let length = u32::from_le_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_RESPONSE_BYTES {
        return Err(SocketError::InvalidResponse("invalid frame length"));
    }
    let mut response_bytes = Zeroizing::new(vec![0u8; length]);
    stream
        .read_exact(&mut response_bytes)
        .await
        .map_err(SocketError::Receive)?;
    let response = PamResponse::from_bytes(&response_bytes)
        .map_err(|_| SocketError::InvalidResponse("malformed payload"))?;
    let _error_message = Zeroizing::new(response.error_message);
    if !response.success {
        // Do not place daemon-controlled response payloads in authentication
        // logs.
        return Err(SocketError::Rejected);
    }

    tracing::debug!("Daemon acknowledged request");
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::{io::AsyncReadExt, net::UnixListener};

    use super::*;

    fn response_frame(success: bool) -> Vec<u8> {
        #[derive(serde::Serialize, zgvariant::Type)]
        struct Response {
            success: bool,
            error_message: String,
        }
        let response = Response {
            success,
            error_message: String::new(),
        };
        let context = zgvariant::serialized::Context::new(zgvariant::LE, 0);
        let bytes = zgvariant::to_bytes(context, &response).unwrap();
        let mut frame = (bytes.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(&bytes);
        frame
    }

    async fn read_request(stream: &mut UnixStream) -> PamMessage {
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).await.unwrap();
        let mut bytes = vec![0; u32::from_le_bytes(length) as usize];
        stream.read_exact(&mut bytes).await.unwrap();
        PamMessage::from_bytes(&bytes).unwrap()
    }

    #[tokio::test]
    async fn test_send_receive() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let listener = UnixListener::bind(&socket_path)?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let message = read_request(&mut stream).await;
            assert_eq!(message.username, "testuser");
            assert_eq!(message.new_secret, b"testpassword");
            stream.write_all(&response_frame(true)).await.unwrap();
        });

        let message = PamMessage::unlock("testuser".to_string(), b"testpassword".to_vec());

        let result = send_secret_to_daemon_async(
            message,
            unsafe { libc::geteuid() },
            false,
            Some(socket_path),
        )
        .await;
        assert!(result.is_ok());

        server.await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_response_failures() -> Result<(), Box<dyn std::error::Error>> {
        let frames = [
            response_frame(false),
            Vec::new(), // EOF without an acknowledgement.
            (MAX_RESPONSE_BYTES as u32 + 1).to_le_bytes().to_vec(),
            vec![3, 0, 0, 0, 1, 0xff, 0], // Invalid UTF-8 in the response string.
        ];
        for (case, frame) in frames.into_iter().enumerate() {
            let temp_dir = tempfile::tempdir()?;
            let socket_path = temp_dir.path().join("test.sock");
            let listener = UnixListener::bind(&socket_path)?;
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _ = read_request(&mut stream).await;
                stream.write_all(&frame).await.unwrap();
            });
            let message = PamMessage::change_password(
                "testuser".to_string(),
                b"old".to_vec(),
                b"new".to_vec(),
            );
            let result = send_secret_to_daemon_async(
                message,
                unsafe { libc::geteuid() },
                false,
                Some(socket_path),
            )
            .await;
            match case {
                0 => assert!(matches!(result, Err(SocketError::Rejected))),
                1 => assert!(matches!(result, Err(SocketError::Receive(_)))),
                _ => assert!(
                    matches!(result, Err(SocketError::InvalidResponse(_))),
                    "case {case}: {result:?}"
                ),
            }
            server.await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_response_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let listener = UnixListener::bind(&socket_path)?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            // Even a partial length prefix must not keep PAM waiting forever.
            stream.write_all(&[1]).await.unwrap();
            std::future::pending::<()>().await;
        });
        let message =
            PamMessage::change_password("testuser".to_string(), b"old".to_vec(), b"new".to_vec());
        let result = send_secret_to_daemon_async(
            message,
            unsafe { libc::geteuid() },
            false,
            Some(socket_path),
        )
        .await;
        assert!(matches!(result, Err(SocketError::Timeout)));
        server.abort();
        let _ = server.await;
        Ok(())
    }
}
