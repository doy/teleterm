pub type Poll<Item, Error> = Result<Async<Item>, Error>;

pub enum Async<Item> {
    // we have a value for the main loop to return immediately.
    Ready(Item),

    // one of our inner futures returned futures::Async::NotReady. if all
    // of our other components return either NothingToDo or NotReady, then our
    // overall future should return NotReady and wait to be polled again.
    NotReady,

    // we did some work (moved our internal state closer to being ready to
    // return a value), but we aren't ready to return a value yet. we should
    // re-run all of the poll functions to see if the state modification made
    // any of them also able to make progress.
    DidWork,

    // we didn't poll any inner futures or otherwise change our internal state
    // at all, so rerunning is unlikely to make progress. if all components
    // return either NothingToDo or NotReady (and at least one returned
    // NotReady), then we should just return NotReady and wait to be polled
    // again. it is an error (panic) for all component poll methods to return
    // NothingToDo.
    NothingToDo,
}

macro_rules! try_ready {
    ($e:expr) => {
        match $e {
            Ok(futures::Async::Ready(t)) => t,
            Ok(futures::Async::NotReady) => {
                return Ok($crate::component_future::Async::NotReady)
            }
            Err(e) => return Err(From::from(e)),
        }
    };
}

pub fn poll_future<T, Item, Error>(
    future: &mut T,
    poll_fns: &'static [&'static dyn for<'a> Fn(
        &'a mut T,
    ) -> Poll<Item, Error>],
) -> futures::Poll<Item, Error>
where
    T: futures::future::Future<Item = Item, Error = Error>,
{
    loop {
        let mut not_ready = false;
        let mut did_work = false;

        for f in poll_fns {
            match f(future)? {
                Async::Ready(e) => return Ok(futures::Async::Ready(e)),
                Async::NotReady => not_ready = true,
                Async::NothingToDo => {}
                Async::DidWork => did_work = true,
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
    ) -> Poll<
        Option<Item>,
        Error,
    >],
) -> futures::Poll<Option<Item>, Error>
where
    T: futures::stream::Stream<Item = Item, Error = Error>,
{
    loop {
        let mut not_ready = false;
        let mut did_work = false;

        for f in poll_fns {
            match f(stream)? {
                Async::Ready(e) => return Ok(futures::Async::Ready(e)),
                Async::NotReady => not_ready = true,
                Async::NothingToDo => {}
                Async::DidWork => did_work = true,
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
