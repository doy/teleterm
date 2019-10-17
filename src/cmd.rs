use crate::prelude::*;

mod play;
mod record;
mod server;
mod stream;
mod watch;

struct Command {
    name: &'static str,
    cmd: &'static dyn for<'a, 'b> Fn(clap::App<'a, 'b>) -> clap::App<'a, 'b>,
    run: &'static dyn for<'a> Fn(&clap::ArgMatches<'a>) -> Result<()>,
    log_level: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "stream",
        cmd: &stream::cmd,
        run: &stream::run,
        log_level: "error",
    },
    Command {
        name: "server",
        cmd: &server::cmd,
        run: &server::run,
        log_level: "info",
    },
    Command {
        name: "watch",
        cmd: &watch::cmd,
        run: &watch::run,
        log_level: "error",
    },
    Command {
        name: "record",
        cmd: &record::cmd,
        run: &record::run,
        log_level: "error",
    },
    Command {
        name: "play",
        cmd: &play::cmd,
        run: &play::run,
        log_level: "error",
    },
];

pub fn parse<'a>() -> Result<clap::ArgMatches<'a>> {
    let mut app = clap::App::new(crate::util::program_name()?)
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
    (chosen_cmd.run)(chosen_submatches)
}
