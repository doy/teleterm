use crate::prelude::*;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    web: crate::config::Web,

    #[serde(
        rename = "oauth",
        deserialize_with = "crate::config::oauth_configs",
        default
    )]
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        std::collections::HashMap<
            crate::protocol::AuthClient,
            crate::oauth::Config,
        >,
    >,
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
    ) -> Box<dyn futures::Future<Item = (), Error = Error> + Send> {
        Box::new(crate::web::Server::new(
            self.web.listen_address,
            self.web.public_address.clone(),
            self.web.server_address.clone(),
            self.web.allowed_login_methods.clone(),
            self.oauth_configs.clone(),
        ))
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
