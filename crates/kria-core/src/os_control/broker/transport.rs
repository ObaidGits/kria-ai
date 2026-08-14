//! Broker transports: the deny-live fakes used by tests and the live socket
//! stub used only in a live composition.
//!
//! linux-os-control-production **Task 1.5**, design §12 (OSC-033).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;
use std::sync::Arc;
use std::time::SystemTime;

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};

use super::client::{BrokerTransport, BrokerTransportError};
use super::server::{CallerContext, LocalBroker};

/// A loopback transport that routes a request frame straight into an in-process
/// [`LocalBroker`] under a fixed caller context and clock. This is the primary
/// fake for full client→broker→client round-trip tests with no socket.
pub struct LoopbackBrokerTransport {
    broker: Arc<LocalBroker>,
    caller: CallerContext,
    now: SystemTime,
}

impl LoopbackBrokerTransport {
    /// Create a loopback transport.
    #[must_use]
    pub fn new(broker: Arc<LocalBroker>, caller: CallerContext, now: SystemTime) -> Self {
        Self {
            broker,
            caller,
            now,
        }
    }
}

impl BrokerTransport for LoopbackBrokerTransport {
    fn round_trip(&self, request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError> {
        match self
            .broker
            .handle_frame(request_frame, &self.caller, self.now)
        {
            Ok(response) => Ok(response),
            // A structurally unbindable frame is dropped by the broker; from the
            // client's perspective the request never produced a bound response.
            Err(_) => Err(BrokerTransportError::ConnectFailed),
        }
    }
}

/// A transport that always reports the request was sent but the response lost.
/// Used to prove post-dispatch transport loss maps to an uncertain outcome.
#[derive(Debug, Clone, Copy, Default)]
pub struct LostAfterSendTransport;

impl BrokerTransport for LostAfterSendTransport {
    fn round_trip(&self, _request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError> {
        Err(BrokerTransportError::LostAfterSend)
    }
}

/// A transport that always fails to connect (pre-dispatch).
#[derive(Debug, Clone, Copy, Default)]
pub struct ConnectFailedTransport;

impl BrokerTransport for ConnectFailedTransport {
    fn round_trip(&self, _request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError> {
        Err(BrokerTransportError::ConnectFailed)
    }
}

/// A transport that returns a fixed, pre-built response frame regardless of the
/// request. Used to feed a tampered response and prove the client rejects a
/// binding that does not echo the request.
pub struct FixedResponseTransport {
    frame: Vec<u8>,
}

impl FixedResponseTransport {
    /// Wrap a fixed response frame.
    #[must_use]
    pub fn new(frame: Vec<u8>) -> Self {
        Self { frame }
    }
}

impl BrokerTransport for FixedResponseTransport {
    fn round_trip(&self, _request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError> {
        Ok(self.frame.clone())
    }
}

/// The live broker transport: a Unix-domain socket to the privileged broker
/// service.
///
/// # Why a Unix socket and not D-Bus activation
///
/// The socket's peer credentials (`SO_PEERCRED`) let the broker learn the caller's
/// uid/pid **from the kernel**, which is what the caller channel binding is
/// derived from. A caller cannot forge them, so the broker never has to trust
/// anything the request itself claims about who sent it.
pub struct LiveBrokerTransport {
    socket_path: PathBuf,
    _seal: (),
}

/// Where the privileged broker listens. Under `/run` because it must not survive
/// a reboot: a stale socket from a previous boot could otherwise be impersonated
/// by an unprivileged process that created the path first.
pub const BROKER_SOCKET_PATH: &str = "/run/kria/broker.sock";

/// Hard cap on a broker response frame. A privileged peer should never send more,
/// and an unbounded read from a socket is a denial-of-service waiting to happen.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// How long the client waits for the privileged service.
const BROKER_TIMEOUT: Duration = Duration::from_secs(30);

impl LiveBrokerTransport {
    /// Construct in a live composition root, using the default socket path.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self {
            socket_path: PathBuf::from(BROKER_SOCKET_PATH),
            _seal: (),
        }
    }

    /// Construct against an explicit socket path.
    #[must_use]
    pub fn with_socket_path(_token: &LiveHostAccessToken, path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
            _seal: (),
        }
    }
}

impl BrokerTransport for LiveBrokerTransport {
    fn round_trip(&self, request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError> {
        // An unbound call cannot produce a matching caller binding, so it is
        // refused rather than sent and rejected by the broker.
        let _ = request_frame;
        Err(BrokerTransportError::ConnectFailed)
    }

    fn round_trip_bound(
        &self,
        request_frame: &[u8],
        connection_nonce: &str,
    ) -> Result<Vec<u8>, BrokerTransportError> {
        deny_live_transport(RawTransportKind::Polkit);

        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|_| BrokerTransportError::ConnectFailed)?;
        // Both directions are bounded: a broker that stops responding must fail
        // the call rather than hang the agent forever holding a lease.
        stream
            .set_read_timeout(Some(BROKER_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(BROKER_TIMEOUT)))
            .map_err(|_| BrokerTransportError::ConnectFailed)?;

        // Nonce preamble: the broker combines this with the kernel's own
        // SO_PEERCRED view to derive the caller binding it compares against the
        // frame. Bounded so a caller cannot make the broker allocate.
        let nonce = connection_nonce.as_bytes();
        if nonce.is_empty() || nonce.len() > 128 {
            return Err(BrokerTransportError::ConnectFailed);
        }
        let nonce_len = u8::try_from(nonce.len()).map_err(|_| BrokerTransportError::ConnectFailed)?;
        stream
            .write_all(&[nonce_len])
            .and_then(|()| stream.write_all(nonce))
            .map_err(|_| BrokerTransportError::ConnectFailed)?;

        // Length-prefixed framing: the broker reads exactly one request per
        // connection, so the length must be explicit rather than implied by EOF.
        let length =
            u32::try_from(request_frame.len()).map_err(|_| BrokerTransportError::ConnectFailed)?;
        // A failure while writing leaves the broker holding an INCOMPLETE frame,
        // which it cannot decode and therefore cannot dispatch — so these stay
        // `ConnectFailed`, which the client maps to "provably no effect".
        stream
            .write_all(&length.to_be_bytes())
            .and_then(|()| stream.write_all(request_frame))
            .map_err(|_| BrokerTransportError::ConnectFailed)?;
        // Once every byte is handed to the kernel the broker may already have
        // acted, so from here on a failure is *uncertain*, never "no effect".
        stream
            .flush()
            .map_err(|_| BrokerTransportError::LostAfterSend)?;
        // Half-close so the broker sees the end of the request without waiting.
        let _ = stream.shutdown(std::net::Shutdown::Write);

        let mut header = [0u8; 4];
        stream
            .read_exact(&mut header)
            .map_err(|_| BrokerTransportError::LostAfterSend)?;
        let response_len = u32::from_be_bytes(header) as usize;
        if response_len == 0 || response_len > MAX_RESPONSE_BYTES {
            // A nonsensical length is a lost response, not a proof of no effect:
            // the privileged operation may already have run.
            return Err(BrokerTransportError::LostAfterSend);
        }
        let mut response = vec![0u8; response_len];
        stream
            .read_exact(&mut response)
            .map_err(|_| BrokerTransportError::LostAfterSend)?;
        Ok(response)
    }
}
