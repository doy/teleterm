use crate::prelude::*;

mod play;
mod record;
mod server;
mod stream;
mod watch;

struct Command {
    name: &'static str,
    cmd: &'static dyn for<'a, 'b> Fn(clap::App<'a, 'b>) -> clap::App<'a, 'b>,
    config: &'static dyn Fn(config::Config) -> Box<dyn crate::config::Config>,
    log_level: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "stream",
        cmd: &stream::cmd,
        config: &stream::config,
        log_level: "error",
    },
    Command {
        name: "server",
        cmd: &server::cmd,
        config: &server::config,
        log_level: "info",
    },
    Command {
        name: "watch",
        cmd: &watch::cmd,
        config: &watch::config,
        log_level: "error",
    },
    Command {
        name: "record",
        cmd: &record::cmd,
        config: &record::config,
        log_level: "error",
    },
    Command {
        name: "play",
        cmd: &play::cmd,
        config: &play::config,
        log_level: "error",
    },
];

pub fn parse<'a>() -> Result<clap::ArgMatches<'a>> {
    let mut app = clap::App::new(program_name()?)
        .about("Stream your terminal for other people to watch")
        .author(clap::crate_authors!())
        .version(clap::crate_version!());

    for cmd in COMMANDS {
        let subcommand = clap::SubCommand::with_name(cmd.name);
        app = app.subcommand((cmd.cmd)(subcommand));
    }

    app.get_matches_safe().context(crate::error::ParseArgs)
}

pub fn run(matches: &clap::ArgMatches<'_>) -> Result<()> {
    let mut chosen_cmd = &COMMANDS[0];
    let mut chosen_submatches = &clap::ArgMatches::<'_>::default();
    for cmd in COMMANDS {
        if let Some(submatches) = matches.subcommand_matches(cmd.name) {
            chosen_cmd = cmd;
            chosen_submatches = submatches;
        }
    }

    env_logger::from_env(
        env_logger::Env::default().default_filter_or(chosen_cmd.log_level),
    )
    .init();

    let config_filename = crate::dirs::Dirs::new().config_file("config.toml");

    let mut config = config::Config::default();
    if let Err(e) = config.merge(config::File::from(config_filename.clone()))
    {
        log::warn!(
            "failed to read config file {}: {}",
            config_filename.to_string_lossy(),
            e
        );
        // if merge returns an error, the config source will still have been
        // added to the config object, so the config object will likely never
        // work, so we should recreate it from scratch.
        config = config::Config::default();
    }
    // as far as i can tell, the Environment source can never actually fail.
    // this is good because figuring out the logic to handle recreating the
    // config object correctly (as per the previous comment) would be quite
    // complicated.
    config
        .merge(config::Environment::with_prefix("TELETERM"))
        .unwrap();

    let mut cmd_config = (chosen_cmd.config)(config);
    cmd_config.merge_args(chosen_submatches)?;
    cmd_config.run()
}

fn program_name() -> Result<String> {
    let program =
        std::env::args().next().context(crate::error::MissingArgv)?;
    let path = std::path::Path::new(&program);
    let filename = path.file_name();
    Ok(filename
        .ok_or_else(|| Error::NotAFileName {
            path: path.to_string_lossy().to_string(),
        })?
        .to_string_lossy()
        .to_string())
}
