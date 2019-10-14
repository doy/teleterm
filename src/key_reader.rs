use crate::prelude::*;

pub struct KeyReader {
    events:
        Option<tokio::sync::mpsc::UnboundedReceiver<crossterm::InputEvent>>,
    quit: Option<tokio::sync::oneshot::Sender<()>>,
}

impl KeyReader {
    pub fn new() -> Self {
        Self {
            events: None,
            quit: None,
        }
    }
}

impl futures::stream::Stream for KeyReader {
    type Item = crossterm::InputEvent;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        if self.events.is_none() {
            let task = futures::task::current();
            let reader = crossterm::input().read_sync();
            let (events_tx, events_rx) =
                tokio::sync::mpsc::unbounded_channel();
            let mut events_tx = events_tx.wait();
            let (quit_tx, mut quit_rx) = tokio::sync::oneshot::channel();
            // TODO: this is pretty janky - it'd be better to build in more
            // useful support to crossterm directly
            std::thread::Builder::new()
                .spawn(move || {
                    for event in reader {
                        // unwrap is unpleasant, but so is figuring out how to
                        // propagate the error back to the main thread
                        events_tx.send(event).unwrap();
                        task.notify();
                        if quit_rx.try_recv().is_ok() {
                            break;
                        }
                    }
                })
                .context(crate::error::TerminalInputReadingThread)?;

            self.events = Some(events_rx);
            self.quit = Some(quit_tx);
        }

        self.events
            .as_mut()
            .unwrap()
            .poll()
            .context(crate::error::ReadChannel)
    }
}

impl Drop for KeyReader {
    fn drop(&mut self) {
        if let Some(quit_tx) = self.quit.take() {
            // don't care if it fails to send, this can happen if the thread
            // terminates due to seeing a newline before the keyreader goes
            // out of scope
            let _ = quit_tx.send(());
        }
    }
}
