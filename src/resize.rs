use crate::prelude::*;

pub struct Resizer {
    winches:
        Box<dyn futures::stream::Stream<Item = (), Error = Error> + Send>,
    sent_initial_size: bool,
}

impl Resizer {
    pub fn new() -> Self {
        let winches = tokio_signal::unix::Signal::new(
            tokio_signal::unix::libc::SIGWINCH,
        )
        .flatten_stream()
        .map(|_| ())
        .context(crate::error::SigWinchHandler);
        Self {
            winches: Box::new(winches),
            sent_initial_size: false,
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl futures::stream::Stream for Resizer {
    type Item = crate::term::Size;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        if !self.sent_initial_size {
            self.sent_initial_size = true;
            return Ok(futures::Async::Ready(
                Some(crate::term::Size::get()?),
            ));
        }
        let _ = futures::try_ready!(self.winches.poll());
        Ok(futures::Async::Ready(Some(crate::term::Size::get()?)))
    }
}

pub enum Event<R: tokio::io::AsyncRead + 'static> {
    Process(<crate::process::Process<R> as futures::stream::Stream>::Item),
    Resize(crate::term::Size),
}

pub struct ResizingProcess<R: tokio::io::AsyncRead + 'static> {
    process: crate::process::Process<R>,
    resizer: Resizer,
}

impl<R: tokio::io::AsyncRead + 'static> ResizingProcess<R> {
    pub fn new(process: crate::process::Process<R>) -> Self {
        Self {
            process,
            resizer: Resizer::new(),
        }
    }
}

impl<R: tokio::io::AsyncRead + 'static> ResizingProcess<R> {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> crate::component_future::Poll<
            Option<Event<R>>,
            Error,
        >] = &[&Self::poll_resize, &Self::poll_process];

    fn poll_resize(
        &mut self,
    ) -> crate::component_future::Poll<Option<Event<R>>, Error> {
        match self.resizer.poll()? {
            futures::Async::Ready(Some(size)) => {
                self.process.resize(size.clone());
                Ok(crate::component_future::Async::Ready(Some(
                    Event::Resize(size),
                )))
            }
            futures::Async::Ready(None) => unreachable!(),
            futures::Async::NotReady => {
                Ok(crate::component_future::Async::NotReady)
            }
        }
    }

    fn poll_process(
        &mut self,
    ) -> crate::component_future::Poll<Option<Event<R>>, Error> {
        match self.process.poll()? {
            futures::Async::Ready(Some(e)) => {
                Ok(crate::component_future::Async::Ready(Some(
                    Event::Process(e),
                )))
            }
            futures::Async::Ready(None) => {
                Ok(crate::component_future::Async::Ready(None))
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Async::NotReady)
            }
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl<R: tokio::io::AsyncRead + 'static> futures::stream::Stream
    for ResizingProcess<R>
{
    type Item = Event<R>;
    type Error =
        <crate::process::Process<R> as futures::stream::Stream>::Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        crate::component_future::poll_stream(self, Self::POLL_FNS)
    }
}
