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
            -> component_future::Poll<
            Option<Event<R>>,
            Error,
        >] = &[&Self::poll_resize, &Self::poll_process];

    fn poll_resize(
        &mut self,
    ) -> component_future::Poll<Option<Event<R>>, Error> {
        let size = component_future::try_ready!(self.resizer.poll()).unwrap();
        self.process.resize(size.clone());
        Ok(component_future::Async::Ready(Some(Event::Resize(size))))
    }

    fn poll_process(
        &mut self,
    ) -> component_future::Poll<Option<Event<R>>, Error> {
        Ok(component_future::Async::Ready(
            component_future::try_ready!(self.process.poll())
                .map(Event::Process),
        ))
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
        component_future::poll_stream(self, Self::POLL_FNS)
    }
}
