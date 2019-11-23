use crate::prelude::*;

use std::io::Write as _;

#[derive(Clone)]
pub struct DiskSession;

impl DiskSession {
    fn file_for_id(
        &self,
        identifier: &gotham::middleware::session::SessionIdentifier,
        should_exist: bool,
    ) -> Option<std::path::PathBuf> {
        let name = format!("web-{}", identifier.value);
        crate::dirs::Dirs::new().data_file(&name, should_exist)
    }
}

impl gotham::middleware::session::NewBackend for DiskSession {
    type Instance = Self;

    fn new_backend(&self) -> std::io::Result<Self::Instance> {
        Ok(Self)
    }
}

impl gotham::middleware::session::Backend for DiskSession {
    fn persist_session(
        &self,
        identifier: gotham::middleware::session::SessionIdentifier,
        content: &[u8],
    ) -> std::result::Result<(), gotham::middleware::session::SessionError>
    {
        let filename = self.file_for_id(&identifier, false).unwrap();
        let mut file = std::fs::File::create(filename).map_err(|e| {
            gotham::middleware::session::SessionError::Backend(format!(
                "{}",
                e
            ))
        })?;
        file.write_all(content).map_err(|e| {
            gotham::middleware::session::SessionError::Backend(format!(
                "{}",
                e
            ))
        })?;
        file.sync_all().map_err(|e| {
            gotham::middleware::session::SessionError::Backend(format!(
                "{}",
                e
            ))
        })?;
        Ok(())
    }

    fn read_session(
        &self,
        identifier: gotham::middleware::session::SessionIdentifier,
    ) -> Box<
        dyn futures::Future<
                Item = Option<Vec<u8>>,
                Error = gotham::middleware::session::SessionError,
            > + Send,
    > {
        if let Some(filename) = self.file_for_id(&identifier, true) {
            Box::new(
                tokio::fs::File::open(filename)
                    .and_then(|file| {
                        let buf = vec![];
                        tokio::io::read_to_end(file, buf)
                    })
                    .map(|(_, v)| Some(v))
                    .map_err(|e| {
                        gotham::middleware::session::SessionError::Backend(
                            format!("{}", e),
                        )
                    }),
            )
        } else {
            Box::new(futures::future::ok(None))
        }
    }

    fn drop_session(
        &self,
        identifier: gotham::middleware::session::SessionIdentifier,
    ) -> std::result::Result<(), gotham::middleware::session::SessionError>
    {
        if let Some(filename) = self.file_for_id(&identifier, true) {
            std::fs::remove_file(filename).map_err(|e| {
                gotham::middleware::session::SessionError::Backend(format!(
                    "{}",
                    e
                ))
            })?;
        }
        Ok(())
    }
}
