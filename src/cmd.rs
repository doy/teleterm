mod cast;
mod server;
mod watch;

use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to determine program name: {}", source))]
    FindProgramName { source: crate::util::Error },

    #[snafu(display("{}", source))]
    Parse { source: clap::Error },

    #[snafu(display("{}", source))]
    Cast { source: crate::cmd::cast::Error },

    #[snafu(display("{}", source))]
    Server { source: crate::cmd::server::Error },

    #[snafu(display("{}", source))]
    Watch { source: crate::cmd::watch::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

struct Command {
    name: &'static str,
    cmd: &'static dyn for<'a, 'b> Fn(clap::App<'a, 'b>) -> clap::App<'a, 'b>,
    run: &'static dyn for<'a> Fn(&clap::ArgMatches<'a>) -> Result<()>,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "cast",
        cmd: &cast::cmd,
        run: &cast::run,
    },
    Command {
        name: "server",
        cmd: &server::cmd,
        run: &server::run,
    },
    Command {
        name: "watch",
        cmd: &watch::cmd,
        run: &watch::run,
    },
];

pub fn parse<'a>() -> Result<clap::ArgMatches<'a>> {
    let mut app =
        clap::App::new(crate::util::program_name().context(FindProgramName)?)
            .about("Broadcast your terminal for other people to watch")
            .author(clap::crate_authors!())
            .version(clap::crate_version!())
            .setting(clap::AppSettings::SubcommandRequired);

    for cmd in COMMANDS {
        let subcommand = clap::SubCommand::with_name(cmd.name);
        app = app.subcommand((cmd.cmd)(subcommand));
    }

    app.get_matches_safe().context(Parse)
}

pub fn run(matches: clap::ArgMatches<'_>) -> Result<()> {
    for cmd in COMMANDS {
        if let Some(submatches) = matches.subcommand_matches(cmd.name) {
            return (cmd.run)(&submatches);
        }
    }
    unreachable!()
}
