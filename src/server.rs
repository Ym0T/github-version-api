//! HTTP server: accept loop with connection limits and timeouts.
//!
//! - A connection must deliver a complete request header within
//!   `first_request_timeout`, whatever protocol it speaks (this also covers
//!   protocol detection and the HTTP/2 preface). HTTP/1 connections must send
//!   every following request header within `header_read_timeout`.
//! - Connections are closed gracefully after `max_connection_age`.
//! - At most `max_connections` in total and `max_connections_per_ip` per
//!   client address (IPv6: per /64) are served; trusted proxies are exempt
//!   from the per-address limit. Excess connections are closed immediately.
//! - On shutdown no new connections are accepted, `before_drain` runs, open
//!   connections finish gracefully for at most `shutdown_grace`.

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tower::ServiceExt;
use tracing::{debug, warn};

use crate::net::{IpNet, rate_limit_key};

#[derive(Debug, Clone)]
pub struct ServerLimits {
    pub first_request_timeout: Duration,
    pub header_read_timeout: Duration,
    pub max_connection_age: Duration,
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
    pub trusted_proxies: Vec<IpNet>,
    pub shutdown_grace: Duration,
}

pub async fn serve(
    listener: TcpListener,
    router: Router,
    limits: ServerLimits,
    shutdown: impl Future<Output = ()>,
    before_drain: impl Future<Output = ()>,
) {
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout);
    builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(30))
        .max_concurrent_streams(64);

    let connections = Arc::new(Semaphore::new(limits.max_connections));
    let per_ip = Arc::new(PerIpLimiter::new(
        limits.max_connections_per_ip,
        limits.trusted_proxies.clone(),
    ));
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::pin!(shutdown);

    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    // E.g. out of file descriptors: back off instead of spinning.
                    warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            debug!(%peer, "connection limit reached, closing connection");
            continue;
        };
        let Some(ip_slot) = PerIpLimiter::try_acquire(&per_ip, peer.ip()) else {
            debug!(%peer, "per-address connection limit reached, closing connection");
            continue;
        };
        tokio::spawn(handle_connection(
            stream,
            peer,
            router.clone(),
            builder.clone(),
            limits.clone(),
            stop_rx.clone(),
            (permit, ip_slot),
        ));
    }

    drop(listener);
    before_drain.await;
    let _ = stop_tx.send(true);
    // All permits are back once every connection has closed.
    let all = u32::try_from(limits.max_connections).unwrap_or(u32::MAX);
    if tokio::time::timeout(limits.shutdown_grace, connections.acquire_many(all))
        .await
        .is_err()
    {
        warn!(
            "open connections did not finish within {}s",
            limits.shutdown_grace.as_secs()
        );
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    router: Router,
    builder: auto::Builder<TokioExecutor>,
    limits: ServerLimits,
    mut stop: watch::Receiver<bool>,
    _guards: (OwnedSemaphorePermit, IpSlot),
) {
    let _ = stream.set_nodelay(true);
    let seen_request = Arc::new(AtomicBool::new(false));
    let service = {
        let seen_request = Arc::clone(&seen_request);
        hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
            seen_request.store(true, Ordering::Relaxed);
            request.extensions_mut().insert(ConnectInfo(peer));
            router.clone().oneshot(request)
        })
    };
    let connection = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    let first_request = tokio::time::sleep(limits.first_request_timeout);
    tokio::pin!(first_request);
    let mut first_request_checked = false;
    let max_age = tokio::time::sleep(limits.max_connection_age);
    tokio::pin!(max_age);
    let mut closing = false;

    loop {
        tokio::select! {
            result = connection.as_mut() => {
                if let Err(e) = result {
                    debug!(%peer, "connection error: {e}");
                }
                return;
            }
            // select! only evaluates the condition when it starts waiting, so
            // whether a request has arrived is checked once the timer fires.
            () = &mut first_request, if !first_request_checked => {
                first_request_checked = true;
                if !seen_request.load(Ordering::Relaxed) {
                    // Dropping the connection closes the socket.
                    debug!(%peer, "no request received in time, closing connection");
                    return;
                }
            }
            () = &mut max_age, if !closing => {
                closing = true;
                connection.as_mut().graceful_shutdown();
            }
            Ok(()) = stop.changed(), if !closing => {
                closing = true;
                connection.as_mut().graceful_shutdown();
            }
        }
    }
}

/// Counts open connections per client address.
struct PerIpLimiter {
    limit: usize,
    trusted_proxies: Vec<IpNet>,
    open: StdMutex<HashMap<IpAddr, usize>>,
}

impl PerIpLimiter {
    fn new(limit: usize, trusted_proxies: Vec<IpNet>) -> Self {
        Self {
            limit,
            trusted_proxies,
            open: StdMutex::new(HashMap::new()),
        }
    }

    fn try_acquire(this: &Arc<Self>, ip: IpAddr) -> Option<IpSlot> {
        if this.trusted_proxies.iter().any(|net| net.contains(ip)) {
            return Some(IpSlot {
                limiter: None,
                key: ip,
            });
        }
        let key = rate_limit_key(ip);
        let mut open = this.open.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(key).or_insert(0);
        if *count >= this.limit {
            return None;
        }
        *count += 1;
        Some(IpSlot {
            limiter: Some(Arc::clone(this)),
            key,
        })
    }
}

struct IpSlot {
    limiter: Option<Arc<PerIpLimiter>>,
    key: IpAddr,
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        let Some(limiter) = &self.limiter else {
            return;
        };
        let mut open = limiter.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;

    use super::*;

    struct TestServer {
        address: SocketAddr,
        stop: Option<oneshot::Sender<()>>,
        done: tokio::task::JoinHandle<()>,
    }

    async fn start(max_connections: usize, per_ip: usize) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route("/", get(|| async { "ok" })).route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(800)).await;
                "slow"
            }),
        );
        let limits = ServerLimits {
            first_request_timeout: Duration::from_millis(300),
            header_read_timeout: Duration::from_millis(300),
            max_connection_age: Duration::from_secs(60),
            max_connections,
            max_connections_per_ip: per_ip,
            trusted_proxies: Vec::new(),
            shutdown_grace: Duration::from_secs(2),
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let done = tokio::spawn(serve(
            listener,
            router,
            limits,
            async move {
                let _ = stopped.await;
            },
            async {},
        ));
        TestServer {
            address,
            stop: Some(stop),
            done,
        }
    }

    /// Waits until the server closes the connection; returns how long it took.
    async fn time_until_closed(stream: &mut TcpStream) -> Duration {
        let start = Instant::now();
        let mut buffer = [0u8; 256];
        loop {
            match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await {
                Ok(Ok(0) | Err(_)) => return start.elapsed(),
                Ok(Ok(_)) => {}
                Err(elapsed) => panic!("connection was not closed: {elapsed}"),
            }
        }
    }

    #[tokio::test]
    async fn serves_requests_and_shuts_down() {
        let mut server = start(8, 8).await;
        let mut stream = TcpStream::connect(server.address).await.unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("ok"));

        server.stop.take().unwrap().send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut server.done)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn slow_responses_outlive_the_first_request_timeout() {
        let server = start(8, 8).await;
        let mut stream = TcpStream::connect(server.address).await.unwrap();
        stream
            .write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("slow"));
    }

    #[tokio::test]
    async fn silent_and_slow_connections_are_closed() {
        let server = start(8, 8).await;
        let silent = async {
            let mut stream = TcpStream::connect(server.address).await.unwrap();
            time_until_closed(&mut stream).await
        };
        let h2_preface = async {
            let mut stream = TcpStream::connect(server.address).await.unwrap();
            stream.write_all(b"PRI * HTTP/2.0\r\n").await.unwrap();
            time_until_closed(&mut stream).await
        };
        let partial_header = async {
            let mut stream = TcpStream::connect(server.address).await.unwrap();
            stream.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
            time_until_closed(&mut stream).await
        };
        let (a, b, c) = tokio::join!(silent, h2_preface, partial_header);
        for elapsed in [a, b, c] {
            assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        }
    }

    #[tokio::test]
    async fn connections_are_limited_per_address() {
        let server = start(8, 2).await;
        let _first = TcpStream::connect(server.address).await.unwrap();
        let _second = TcpStream::connect(server.address).await.unwrap();
        // Give the accept loop time to register both connections.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut third = TcpStream::connect(server.address).await.unwrap();
        let elapsed = time_until_closed(&mut third).await;
        assert!(
            elapsed < Duration::from_millis(200),
            "closed immediately, took {elapsed:?}"
        );
    }
}
