//! Resilience of the public player under hostile sources and lifecycle churn.
//!
//! The tests here go through public entry points only. Two mock decoders drive
//! the thread boundary without a platform codec, and a one-shot HTTP server
//! built on `std::net::TcpListener` drives the HLS failure path without a new
//! dependency or an outside network.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use cranpose_video::{DecodedFrame, VideoDecoder, VideoError, VideoInfo, VideoPlayer, VideoState};

/// How long a state change gets before the test calls it stuck.
const TIMEOUT: Duration = Duration::from_secs(15);

fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    ready()
}

/// Whether this build carries the in-crate HLS reader the two HTTP tests need.
///
/// The same entry point exists elsewhere, but a build without a reader falls
/// back to the platform decoder and never builds the HLS error this file checks.
fn hls_reader() -> bool {
    cfg!(any(
        target_os = "windows",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        all(target_os = "linux", feature = "linux-gstreamer")
    ))
}

/// A decoder whose media is empty from the first read onward.
struct EmptyDecoder;

impl VideoDecoder for EmptyDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Ok(())
    }

    fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        Ok(None)
    }
}

/// A decoder that panics in the two calls the worker makes when it leaves.
struct DoublePanicDecoder;

impl VideoDecoder for DoublePanicDecoder {
    fn info(&self) -> VideoInfo {
        VideoInfo {
            width: 16,
            height: 16,
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, _position: Duration) -> Result<(), VideoError> {
        Ok(())
    }

    fn read(&mut self, _rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        panic!("read exploded");
    }

    fn close(&mut self) {
        panic!("close exploded");
    }
}

/// What the local server answers to every request it receives.
#[derive(Clone, Copy)]
enum Reply {
    /// A forbidden address, which the reader treats as permanent.
    Forbidden,
    /// A 200 with an empty body, from which no playlist can be parsed.
    EmptyPlaylist,
}

/// An in-process HTTP server with one canned reply for every request.
///
/// The listener lives on the worker so the port is released when the stop flag
/// ends it. Replies are small and carry `Content-Length`, so a request never
/// waits on a keep-alive stream the test does not read.
struct TestServer {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
        let port = listener.local_addr().expect("a local address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            listener.set_nonblocking(true).expect("nonblocking accept");
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut request = [0_u8; 4096];
                        let _ = stream.read(&mut request);
                        let response: &[u8] = match reply {
                            Reply::Forbidden => {
                                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            }
                            Reply::EmptyPlaylist => {
                                b"HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            }
                        };
                        let _ = stream.write_all(response);
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            // The query carries a token so a leaked URL is visible in the error.
            url: format!("http://127.0.0.1:{port}/live/index.m3u8?token=secret-token-123"),
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
fn an_exhausted_decoder_settles_to_a_terminal_state() {
    let player = VideoPlayer::with_decoder(EmptyDecoder).expect("the player opens");
    assert_ne!(
        player.state(),
        VideoState::Loading,
        "a decoder that states its size starts paused"
    );
    player.play().expect("playback starts");
    assert!(
        wait_until(|| matches!(player.state(), VideoState::Ended | VideoState::Failed(_))),
        "an exhausted decoder never left {:?}",
        player.state()
    );
    assert!(
        player.state().is_ended(),
        "an exhausted decoder ends the feed, not stays in {:?}",
        player.state()
    );
    // It stays terminal while the screen keeps asking for pictures.
    for _ in 0..20 {
        let _ = player.take_frame();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        player.state().is_ended(),
        "pumping pictures restarted a finished feed: {:?}",
        player.state()
    );
}

#[test]
fn open_hls_does_not_panic_or_stall_on_an_empty_playlist() {
    if !hls_reader() {
        return;
    }
    let server = TestServer::start(Reply::EmptyPlaylist);
    // `open_hls` only remembers the address; a panic here or in the first read
    // fails this test, which is the assertion that malformed input fails closed.
    let mut player = VideoPlayer::open_hls(server.url.clone()).expect("the address is remembered");
    player.play().expect("playback starts");
    assert!(
        wait_until(|| matches!(player.state(), VideoState::Failed(_) | VideoState::Ended)),
        "an empty playlist left the player in {:?}",
        player.state()
    );
    player.close();
    assert!(player.is_closed());
}

#[test]
fn open_hls_turns_a_forbidden_playlist_into_a_redacted_error() {
    if !hls_reader() {
        return;
    }
    let server = TestServer::start(Reply::Forbidden);
    let mut player = VideoPlayer::open_hls(server.url.clone()).expect("the address is remembered");
    player.play().expect("playback starts");
    assert!(
        wait_until(|| matches!(player.state(), VideoState::Failed(_))),
        "a forbidden playlist left the player in {:?}",
        player.state()
    );

    let error = match player.state() {
        VideoState::Failed(error) => error,
        other => panic!("a 403 must be a failure, and the state is {other:?}"),
    };
    assert!(
        matches!(error, VideoError::Decode(_) | VideoError::Source(_)),
        "a 403 must state why it failed: {error:?}"
    );
    let message = error
        .detail()
        .expect("a decode or source failure states its reason")
        .to_owned();
    assert!(message.contains("403"), "the status is missing: {message}");
    assert!(
        message.contains("/live/index.m3u8"),
        "the source is not attributable: {message}"
    );
    assert!(
        message.contains("?<redacted>"),
        "the query is not marked as redacted: {message}"
    );
    assert!(
        !message.contains("secret-token-123") && !message.contains("token="),
        "the token leaked into the public error: {message}"
    );
    player.close();
}

#[test]
fn close_is_idempotent() {
    let mut player = VideoPlayer::with_decoder(EmptyDecoder).expect("the player opens");
    player.play().expect("playback starts");
    assert!(
        wait_until(|| player.state().is_ended()),
        "the decoder never finished"
    );
    let began = Instant::now();
    player.close();
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "close blocked for {:?}",
        began.elapsed()
    );
    // Repeating the call, including the one `Drop` will make, must do nothing.
    player.close();
    player.close();
    assert!(player.is_closed());
    assert!(player.take_frame().is_none());
    assert_eq!(player.play(), Err(VideoError::Closed));
    assert_eq!(player.pause(), Err(VideoError::Closed));
}

#[test]
fn a_panic_in_read_then_close_does_not_unwind_past_the_worker() {
    let mut player = VideoPlayer::with_decoder(DoublePanicDecoder).expect("the player opens");
    assert!(
        wait_until(|| player.state().failure().is_some()),
        "the read panic never reached the failure state"
    );
    let message = player
        .state()
        .failure()
        .and_then(VideoError::detail)
        .unwrap_or_default()
        .to_owned();
    assert!(
        message.contains("read exploded"),
        "the read panic was not reported: {message}"
    );
    // The worker unwinds out of `read` and then out of `close` when it leaves.
    // Neither unwind may reach this thread or keep `close` from returning.
    let began = Instant::now();
    player.close();
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "close blocked for {:?} after the double panic",
        began.elapsed()
    );
    player.close();
    assert!(player.is_closed());
}

#[test]
fn many_open_close_cycles_do_not_hang() {
    // This host has no /proc to sample, so the portable assertion is that every
    // cycle finishes well inside a budget and the next one can still start.
    const CYCLES: usize = 100;
    const BUDGET: Duration = Duration::from_secs(20);
    let (done_tx, done_rx) = mpsc::channel();
    let runner = thread::spawn(move || {
        for cycle in 0..CYCLES {
            let mut player = match VideoPlayer::with_decoder(EmptyDecoder) {
                Ok(player) => player,
                Err(error) => {
                    let _ = done_tx.send(Err(format!("cycle {cycle}: open: {error}")));
                    return;
                }
            };
            if let Err(error) = player.play() {
                let _ = done_tx.send(Err(format!("cycle {cycle}: play: {error}")));
                return;
            }
            let _ = player.take_frame();
            let began = Instant::now();
            player.close();
            if began.elapsed() > Duration::from_secs(2) {
                let _ = done_tx.send(Err(format!(
                    "cycle {cycle}: close took {:?}",
                    began.elapsed()
                )));
                return;
            }
            if !player.is_closed() {
                let _ = done_tx.send(Err(format!("cycle {cycle}: still open")));
                return;
            }
        }
        let _ = done_tx.send(Ok(()));
    });
    let outcome = done_rx
        .recv_timeout(BUDGET)
        .expect("the open/close cycles hung");
    runner.join().expect("the cycle runner panicked");
    outcome.expect("a cycle reported a failure");
}
