struct EventedStdin;

const STDIN: i32 = 0;

impl std::io::Read for EventedStdin {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        stdin.read(buf)
    }
}

impl mio::Evented for EventedStdin {
    fn register(
        &self,
        poll: &mio::Poll,
        token: mio::Token,
        interest: mio::Ready,
        opts: mio::PollOpt,
    ) -> std::io::Result<()> {
        let fd = STDIN as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &mio::Poll,
        token: mio::Token,
        interest: mio::Ready,
        opts: mio::PollOpt,
    ) -> std::io::Result<()> {
        let fd = STDIN as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> std::io::Result<()> {
        let fd = STDIN as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.deregister(poll)
    }
}

pub struct Stdin {
    input: tokio::reactor::PollEvented2<EventedStdin>,
}

impl Stdin {
    pub fn new() -> Self {
        Self {
            input: tokio::reactor::PollEvented2::new(EventedStdin),
        }
    }
}

impl std::io::Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buf)
    }
}

impl tokio::io::AsyncRead for Stdin {
    fn poll_read(
        &mut self,
        buf: &mut [u8],
    ) -> std::result::Result<futures::Async<usize>, tokio::io::Error> {
        // XXX this is why i had to do the EventedFd thing - poll_read on its
        // own will block reading from stdin, so i need a way to explicitly
        // check readiness before doing the read
        let ready = mio::Ready::readable();
        match self.input.poll_read_ready(ready)? {
            futures::Async::Ready(_) => {
                let res = self.input.poll_read(buf);

                // XXX i'm pretty sure this is wrong (if the single poll_read
                // call didn't return all waiting data, clearing read ready
                // state means that we won't get the rest until some more data
                // beyond that appears), but i don't know that there's a way
                // to do it correctly given that poll_read blocks
                self.input.clear_read_ready(ready)?;

                res
            }
            futures::Async::NotReady => Ok(futures::Async::NotReady),
        }
    }
}
