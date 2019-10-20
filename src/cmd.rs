use crate::prelude::*;

mod play;
mod record;
mod server;
mod stream;
mod watch;

struct Command {
    name: &'static str,
    cmd: &'static dyn for<'a, 'b> Fn(clap::App<'a, 'b>) -> clap::App<'a, 'b>,
    config: &'static dyn Fn(
        Option<config::Config>,
    ) -> Result<Box<dyn crate::config::Config>>,
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

    let config = crate::config::config()?;
    let mut cmd_config = (chosen_cmd.config)(config)?;
    cmd_config.merge_args(chosen_submatches)?;
    log::debug!("{:?}", cmd_config);
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
