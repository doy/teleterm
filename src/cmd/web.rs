use crate::prelude::*;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        _matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        Ok(())
    }

    fn run(
        &self,
    ) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send>
    {
        let addr = "127.0.0.1:4145";
        Box::new(
            gotham::init_server(addr, crate::web::router())
                .map_err(|_| unreachable!()),
        )
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Run a teleterm web server")
}

pub fn config(
    config: Option<config::Config>,
) -> Result<Box<dyn crate::config::Config>> {
    let config: Config = if let Some(config) = config {
        config
            .try_into()
            .context(crate::error::CouldntParseConfig)?
    } else {
        Config::default()
    };
    Ok(Box::new(config))
}
