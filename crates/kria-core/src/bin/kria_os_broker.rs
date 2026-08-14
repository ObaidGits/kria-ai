//! `kria-os-broker` — the small privileged service.
//!
//! linux-os-control-production **Task 1.5**, design §12.
//!
//! # What this process is for
//!
//! A handful of OS changes genuinely need root: changing a file's owner, writing
//! battery charge thresholds. Rather than let the whole agent run privileged,
//! **only this process does** — and it accepts exactly one narrow, typed request
//! per connection, authorizes it through Polkit, performs one fixed operation, and
//! exits that connection.
//!
//! # Trust model
//!
//! * The caller's uid/gid/pid come from the **kernel** (`SO_PEERCRED`), never from
//!   the request body, so a caller cannot claim to be someone else.
//! * The request carries a caller binding; this process derives the same binding
//!   independently and rejects a mismatch **before** Polkit or any effect.
//! * Authorization is Polkit's decision, not this process's. There is no
//!   "trusted caller" bypass and no configuration that grants one.
//! * A nonce replay store makes a captured request single-use.
//! * There is no shell, no command string, and no caller-supplied path in any
//!   privileged action — only closed typed operations.
//!
//! # Running it
//!
//! This binary must run as root to be useful, and it is **not** installed or
//! started automatically. See `deploy/broker/README.md` for the systemd unit and
//! Polkit policy, which a human installs deliberately.

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use kria_core::os_control::broker::{
    CallerContext, LocalBroker, PeerCredentials, BROKER_SOCKET_PATH,
};

/// Hard cap on a request frame. The protocol's own bound is smaller; this simply
/// stops a hostile peer from making the privileged process allocate.
const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// Per-connection I/O bound. A stalled peer must not hold a privileged worker.
const IO_TIMEOUT: Duration = Duration::from_secs(15);

fn main() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "kria-os-broker: refusing to start unprivileged — it would deny every \
             request it accepted, which is worse than not running at all"
        );
        std::process::exit(1);
    }

    let socket_path = std::env::var("KRIA_BROKER_SOCKET")
        .unwrap_or_else(|_| BROKER_SOCKET_PATH.to_string());
    let path = Path::new(&socket_path);

    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            eprintln!("kria-os-broker: cannot create {}", parent.display());
            std::process::exit(1);
        }
    }
    // Remove a stale socket from a previous run. Under /run this cannot be a
    // leftover from a previous boot, so there is nothing to preserve.
    let _ = std::fs::remove_file(path);

    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("kria-os-broker: cannot bind {socket_path}: {error}");
            std::process::exit(1);
        }
    };
    // World-writable so any local session can *ask*. This is safe because
    // authorization is Polkit's decision, not the socket's permissions —
    // restricting the socket would only hide requests, not secure them.
    if let Err(error) = set_socket_permissions(path) {
        eprintln!("kria-os-broker: cannot set socket permissions: {error}");
        std::process::exit(1);
    }

    let broker = Arc::new(build_broker());
    eprintln!("kria-os-broker: listening on {socket_path}");

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                // One request per connection, handled inline. The work is a single
                // bounded syscall or a Polkit round trip, so a thread per
                // connection would add risk (shared privileged state) without
                // meaningful throughput gain.
                if let Err(error) = serve_one(&broker, stream) {
                    eprintln!("kria-os-broker: connection error: {error}");
                }
            }
            Err(error) => eprintln!("kria-os-broker: accept failed: {error}"),
        }
    }
}

fn set_socket_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))
}

fn build_broker() -> LocalBroker {
    // The live composition mints the host-access token; the deny-live sentinel is
    // inert here because this binary is built without `os-control-test`.
    let token = kria_core::os_control::access::LiveHostAccessToken::mint();
    // In-memory replay store: a restart forgets nonces, so a captured request
    // could in principle be replayed across a restart. Grant expiry is the
    // backstop — a replayed request outside its grant window is refused as
    // expired before Polkit or any effect. A persistent store would close the
    // remaining window and is the obvious hardening step.
    LocalBroker::new(
        Arc::new(kria_core::os_control::broker::InMemoryNonceStore::default()),
        Arc::new(kria_core::os_control::broker::LivePolkitAuthorizer::new(&token)),
        Arc::new(kria_core::os_control::broker::LiveNativeOperations::new(&token)),
    )
}

/// Read the kernel's view of who is on the other end of the socket.
fn peer_credentials(stream: &UnixStream, nonce: String) -> Option<PeerCredentials> {
    let mut creds: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `stream` is an open socket; `creds`/`len` are valid out-pointers
    // sized exactly as the kernel expects.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(creds).cast::<libc::c_void>(),
            &mut len,
        )
    };
    if result != 0 {
        return None;
    }
    Some(PeerCredentials {
        uid: creds.uid,
        gid: creds.gid,
        pid: creds.pid,
        connection_nonce: nonce,
    })
}

fn serve_one(broker: &LocalBroker, mut stream: UnixStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    // 1. Nonce preamble.
    let mut nonce_len = [0u8; 1];
    stream.read_exact(&mut nonce_len)?;
    let mut nonce_bytes = vec![0u8; nonce_len[0] as usize];
    stream.read_exact(&mut nonce_bytes)?;
    let Ok(nonce) = String::from_utf8(nonce_bytes) else {
        // A non-UTF-8 nonce cannot have produced the caller's binding.
        return Ok(());
    };

    // 2. Length-prefixed request frame.
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let frame_len = u32::from_be_bytes(header) as usize;
    if frame_len == 0 || frame_len > MAX_REQUEST_BYTES {
        return Ok(());
    }
    let mut frame = vec![0u8; frame_len];
    stream.read_exact(&mut frame)?;

    // 3. The caller identity the KERNEL reports, combined with the nonce.
    let caller = match peer_credentials(&stream, nonce) {
        Some(peer) => CallerContext::authenticated(peer),
        // Without kernel-supplied credentials there is no authenticated caller,
        // and the broker refuses rather than assuming one.
        None => CallerContext::unauthenticated(),
    };

    // 4. One decision, one fixed operation. Every refusal path inside returns a
    //    bound response, so the client can always tell "refused" from "unknown".
    match broker.handle_frame(&frame, &caller, SystemTime::now()) {
        Ok(response) => {
            let length = u32::try_from(response.len()).unwrap_or(0);
            stream.write_all(&length.to_be_bytes())?;
            stream.write_all(&response)?;
            stream.flush()?;
        }
        // A structurally undecodable frame has no binding to echo, so there is
        // nothing meaningful to reply; the client's read fails and it reports the
        // call as lost rather than as a decision.
        Err(_structural) => {}
    }
    Ok(())
}
