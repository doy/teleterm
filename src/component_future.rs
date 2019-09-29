pub enum Poll<T> {
    // something happened that we want to report
    Event(T),
    // underlying future/stream returned NotReady, so it's safe for us to also
    // return NotReady
    NotReady,
    // didn't do any work, so we want to return NotReady assuming at least one
    // other poll method returned NotReady (if every poll method returns
    // NothingToDo, something is broken)
    NothingToDo,
    // did some work, so we want to loop
    DidWork,
    // the stream has ended
    Done,
}

pub fn poll_future<T, Item, Error>(
    future: &mut T,
    poll_fns: &'static [&'static dyn for<'a> Fn(
        &'a mut T,
    ) -> Result<
        Poll<Item>,
        Error,
    >],
) -> futures::Poll<Item, Error> {
    loop {
        let mut not_ready = false;
        let mut did_work = false;

        for f in poll_fns {
            match f(future)? {
                Poll::Event(e) => return Ok(futures::Async::Ready(e)),
                Poll::NotReady => not_ready = true,
                Poll::NothingToDo => {}
                Poll::DidWork => did_work = true,
                Poll::Done => unreachable!(),
            }
        }

        if !did_work {
            if not_ready {
                return Ok(futures::Async::NotReady);
            } else {
                unreachable!()
            }
        }
    }
}

pub fn poll_stream<T, Item, Error>(
    stream: &mut T,
    poll_fns: &'static [&'static dyn for<'a> Fn(
        &'a mut T,
    ) -> Result<
        Poll<Item>,
        Error,
    >],
) -> futures::Poll<Option<Item>, Error> {
    loop {
        let mut not_ready = false;
        let mut did_work = false;

        for f in poll_fns {
            match f(stream)? {
                Poll::Event(e) => return Ok(futures::Async::Ready(Some(e))),
                Poll::NotReady => not_ready = true,
                Poll::NothingToDo => {}
                Poll::DidWork => did_work = true,
                Poll::Done => return Ok(futures::Async::Ready(None)),
            }
        }

        if !did_work {
            if not_ready {
                return Ok(futures::Async::NotReady);
            } else {
                unreachable!()
            }
        }
    }
}
