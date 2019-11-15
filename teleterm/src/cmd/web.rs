use crate::prelude::*;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    web: crate::config::Web,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.web.merge_args(matches)
    }

    fn run(
        &self,
    ) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send>
    {
        Box::new(
            gotham::init_server(
                self.web.listen_address,
                crate::web::router(),
            )
            .map_err(|_| unreachable!()),
        )
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Web::cmd(app.about("Run a teleterm web server"))
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
