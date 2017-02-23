//! The `PingPong` protocol middleware.
use frame::WebSocket;
use futures::{Async, AsyncSink, Poll, Sink, StartSend, Stream};
use slog::Logger;
use std::collections::VecDeque;
use std::io;
use util;

/// The `PingPong` struct.
pub struct PingPong<T> {
    /// The upstream protocol.
    upstream: T,
    /// A vector of app datas for the given pings.  A pong is sent with the same data.
    app_datas: VecDeque<Option<Vec<u8>>>,
    /// slog stdout `Logger`
    stdout: Option<Logger>,
    /// slog stderr `Logger`
    stderr: Option<Logger>,
}

impl<T> PingPong<T> {
    /// Create a new `PingPong` protocol middleware.
    pub fn new(upstream: T) -> PingPong<T> {
        PingPong {
            upstream: upstream,
            app_datas: VecDeque::new(),
            stdout: None,
            stderr: None,
        }
    }

    /// Add a stdout slog `Logger` to this protocol.
    pub fn stdout(&mut self, logger: Logger) -> &mut PingPong<T> {
        let stdout = logger.new(o!("proto" => "pingpong"));
        self.stdout = Some(stdout);
        self
    }

    /// Add a stderr slog `Logger` to this protocol.
    pub fn stderr(&mut self, logger: Logger) -> &mut PingPong<T> {
        let stderr = logger.new(o!("proto" => "pingpong"));
        self.stderr = Some(stderr);
        self
    }
}

impl<T> Stream for PingPong<T>
    where T: Stream<Item = WebSocket, Error = io::Error>,
          T: Sink<SinkItem = WebSocket, SinkError = io::Error>
{
    type Item = WebSocket;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<WebSocket>, io::Error> {
        loop {
            match try_ready!(self.upstream.poll()) {
                Some(ref msg) if msg.is_pong() => {
                    // Eat pongs
                    try!(self.poll_complete());
                }
                Some(ref msg) if msg.is_ping() => {
                    if let Some(base) = msg.base() {
                        self.app_datas.push_back(base.application_data().cloned());
                    } else {
                        return Err(util::other("couldn't extract base frame"));
                    }

                    try!(self.poll_complete());
                }
                m => return Ok(Async::Ready(m)),
            }
        }
    }
}

impl<T> Sink for PingPong<T>
    where T: Sink<SinkItem = WebSocket, SinkError = io::Error>
{
    type SinkItem = WebSocket;
    type SinkError = io::Error;

    fn start_send(&mut self, item: WebSocket) -> StartSend<WebSocket, io::Error> {
        if !self.app_datas.is_empty() {
            try_warn!(self.stdout, "sink has pending pings");
            return Ok(AsyncSink::NotReady(item));
        }

        self.upstream.start_send(item)
    }

    fn poll_complete(&mut self) -> Poll<(), io::Error> {
        let mut cur = self.app_datas.pop_front();
        while let Some(app_data) = cur {
            let pong = WebSocket::pong(app_data);
            let res = try!(self.upstream.start_send(pong));

            if res.is_ready() {
                try_trace!(self.stdout, "pong message sent");
            } else {
                break;
            }
            cur = self.app_datas.pop_front();
        }

        self.upstream.poll_complete()
    }
}
