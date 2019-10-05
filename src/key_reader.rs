use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display(
        "failed to spawn a background thread to read terminal input: {}",
        source
    ))]
    TerminalInputReadingThread { source: std::io::Error },

    #[snafu(display("failed to read from event channel: {}", source))]
    ReadChannel {
        source: tokio::sync::mpsc::error::UnboundedRecvError,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct KeyReader {
    events: tokio::sync::mpsc::UnboundedReceiver<crossterm::InputEvent>,
    quit: Option<tokio::sync::oneshot::Sender<()>>,
}

impl KeyReader {
    pub fn new(task: futures::task::Task) -> Result<Self> {
        let reader = crossterm::input().read_sync();
        let (mut events_tx, events_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (quit_tx, mut quit_rx) = tokio::sync::oneshot::channel();
        // TODO: this is pretty janky - it'd be better to build in more useful
        // support to crossterm directly
        std::thread::Builder::new()
            .spawn(move || {
                for event in reader {
                    // unwrap is unpleasant, but so is figuring out how to
                    // propagate the error back to the main thread
                    events_tx.try_send(event).unwrap();
                    task.notify();
                    if quit_rx.try_recv().is_ok() {
                        break;
                    }
                }
            })
            .context(TerminalInputReadingThread)?;

        Ok(Self {
            events: events_rx,
            quit: Some(quit_tx),
        })
    }
}

impl futures::stream::Stream for KeyReader {
    type Item = crossterm::InputEvent;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        self.events.poll().context(ReadChannel)
    }
}

impl Drop for KeyReader {
    fn drop(&mut self) {
        // don't care if it fails to send, this can happen if the thread
        // terminates due to seeing a newline before the keyreader goes out of
        // scope
        let quit_tx = self.quit.take();
        let _ = quit_tx.unwrap().send(());
    }
}
