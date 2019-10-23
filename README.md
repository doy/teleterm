# teleterm

## Overview

When I was first learning to program, one of the things I did in my spare time
was play NetHack. In particular, I played on the
[nethack.alt.org](https://alt.org/nethack/) public server, and hung out in
\#nethack on IRC. One of the things that made this a great learning environment
was that all games played on this server are automatically recorded and
livestreamed. This allowed you to both watch other people play to pick up tips,
as well as ask other people to look at your game and give you advice.

After a while, a group of us realized that this model could be used for more
than just playing games, and set up a similar terminal re-broadcaster for
general purpose use. This allowed us to see what other peoples' development
environments and workflows were like in real time, and made collaborating on
projects much more seamless. `teleterm` is an attempt to recreate that
environment that was so helpful in my own learning process, while fixing some
of the issues that the original version had.

In particular, `teleterm` is intended to be able to be run entirely
transparently (you shouldn't even know it's running while you're streaming),
and you should be able to keep a window open to watch other peoples' terminals
in the corner of your screen without it being disruptive. `teleterm` doesn't
include any functionality to control your local terminal remotely, and doesn't
include any communication functionality (other than the terminal itself) - it
is best used in an already existing community with more featureful
communication methods.

## Features

* Transparently broadcast your terminal session, optionally using TLS
  encryption and secure authentication
* Automatically reconnect in the background when you lose internet
  connectivity, without the work you're doing in your terminal session being
  disrupted
* Record and play back [ttyrec](https://en.wikipedia.org/wiki/Ttyrec) files

## Installation

If you have a working [rust](https://www.rust-lang.org/) installation,
`teleterm` can be installed from source by running `cargo install teleterm`.
Otherwise, we provide prebuilt packages for a couple operating systems:

### [Arch Linux](https://git.tozt.net/teleterm/releases/arch/)

### [Ubuntu/Debian](https://git.tozt.net/teleterm/releases/deb/)

All packages are signed, and can be verified with
[minisign](https://jedisct1.github.io/minisign/) using the public key
`RWTM0AZ5RpROOfAIWx1HvYQ6pw1+FKwN6526UFTKNImP/Hz3ynCFst3r`.

## Usage

### Streaming

You can start streaming by simply running `tt` (or `tt stream`). It will prompt
you for some information about the server you would like to connect to, and
store that information in a configuration file in your home directory. (Note
that I am not running any publically accessible server, because I believe this
works better as a tool for smaller, already existing communities, so you'll
need to run your own or find someone else to host one first.)

### Watching

To watch existing streams, run `tt watch`. This will display a menu of
currently active streams - select one, and it will be displayed in your
terminal. Press `q` to return to the menu.

### Recording

You can record your terminal session to a file by running `tt record`. This
uses the standard [ttyrec](https://en.wikipedia.org/wiki/Ttyrec) file format,
which can be understood by many different applications (including `tt play`).
Note that both `tt stream` and `tt record` can be given a command to run
instead of just a shell, so you can broadcast your terminal and record the
session to a file at once by running `tt stream tt record`.

### Playback

You can play back previously recorded ttyrec files by using `tt play`.

## Configuration

### Command line flags

These are documented via `tt help`.

### Environment variables

`tt` respects the [`RUST_LOG`](https://docs.rs/env_logger/*/env_logger/)
environment variable to adjust the logging verbosity. By default, `tt server`
displays logs at the `info` level and the rest of the commands display logs at
the `error` level, but you can run a command like `RUST_LOG=tt=info tt stream`
to see more information. Note that for interactive commands like `tt stream`,
this will likely be disruptive, but you can send the output to a file by
redirecting `STDERR` (since all process output is written to `tt`'s `STDOUT`
and all log output is written to `tt`'s `STDERR`), like this: `RUST_LOG=tt=info
tt stream 2>>stream.log`.

### Configuration file

`teleterm` also optionally reads configuration from a configuration file. This
file should be in [TOML](https://en.wikipedia.org/wiki/TOML) format, and stored
either in `~/.config/teleterm/config.toml` or `/etc/teleterm/config.toml`. If a
configuration file does not exist, `tt stream` and `tt watch` will offer to
create one for you automatically. The configuration has several sections:

#### `[server]` (used by `tt server`)

* `listen_address`
    * Local address for the server to listen on, in the format `HOST:PORT`.
    * Default: `127.0.0.1:4144`
* `buffer_size`
    * Maximum size of the per-connection buffer to maintain, which will be sent
      when a new client connects (in order to be able to fully redraw the
      current terminal state).
    * Default: `4194304`
* `read_timeout`
    * Amount of time in seconds to wait without receiving data from a client
      before disconnecting that client. Note that besides sending data on
      terminal output, clients also send a heartbeat message every 30 seconds
      in order to keep the connection alive.
    * Default: `120`
* `tls_identity_file`
    * If this option is specified, the server will use TLS to encrypt incoming
      connections (and clients connecting to this server must enable the `tls`
      client option). The value of this option should be the path to a file
      containing the TLS private key along with a certificate chain up to a
      trusted root, in PKCS #12 format. This file can be generated from an
      existing private key and cert chain using a command like this:
      ```
      openssl pkcs12 -export -out identity.pfx -inkey key.pem -in cert.pem -certfile chain_certs.pem
      ```
    * Default: unset (the server will accept plaintext TCP connections)
* `allowed_login_methods`
    * List of login methods to allow from incoming connections. Must be
      non-empty. Valid login methods are:
        * `plain`: The client supplies a username, which the server uses
          directly. Allows impersonation, but can be fine if that's not an
          issue for you.
        * `recurse_center`: The client authenticates via the
          [Recurse Center](https://www.recurse.com/)'s OAuth flow, and
          retrieves the user's name from the Recurse Center API.
    * Default: `["plain", "recurse_center"]`
* `uid`
    * If set and the server is run as `root`, the server will switch to this
      username or uid after binding to a port and reading the TLS key. This
      allows you to use a low-numbered port or a `root`-owned TLS key without
      requiring the server itself to handle connection requests as `root`.
    * Default: unset
* `gid`
    * Same as `uid`, except sets the user's primary group.
    * Default: unset

#### `[oauth.<method>]` (used by `tt server`)

`<method>` corresponds to an OAuth-using login method - for instance, a section
would be named something like `[oauth.recurse_center]`. Note that OAuth login
methods are required to use `http://localhost:44141` as their redirect URL.

* `client_id`
    * OAuth client id.
* `client_secret`
    * OAuth client secret.

#### `[client]` (used by `tt stream` and `tt watch`)

* `auth`
    * Login method to use (must be one of the methods that the server has been
      configured to accept).
    * Default: `plain`
* `username`
    * If using the `plain` login method, the username to log in as.
    * Default: the local username that the `tt` process is running under
      (fetched from the `$USER` environment variable)
* `connect_address`
    * Address to connect to, in `HOST:PORT` form. Note that when connecting to
      a TLS-using server, the `HOST` component must correspond to a name on the
      TLS certificate used by the server.
    * Default: `127.0.0.1:4144`
* `tls`
    * Whether to connect to the server using TLS.
    * Default: `false`

#### `[command]` (used by `tt stream` and `tt record`)

* `buffer_size`
    * Maximum size of the buffer to maintain, which will be sent to the server
      when reconnecting after a connection drops (in order to be able to fully
      redraw the current terminal state).
    * Default: `4194304`
* `command`
    * Command to execute.
    * Default: the currently running shell (fetched from the `$SHELL`
      environment variable)
* `args`
    * List of arguments to pass to `command`.
    * Default: `[]`

#### `[ttyrec]` (used by `tt record` and `tt play`)

* `filename`
    * Name of the TTYrec file to save to or read from.
    * Default: `teleterm.ttyrec`

## Contributing

I'm very interested in contributions! I have a list of todo items in this
repository at TODO.md, but I'm also open to any other patches you think would
make this more useful. Send me an email, or open a ticket or pull request on
Github or Gitlab.
