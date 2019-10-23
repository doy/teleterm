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

## Contributing

I'm very interested in contributions! I have a list of todo items in this
repository at TODO.md, but I'm also open to any other patches you think would
make this more useful. Send me an email, or open a ticket or pull request on
Github or Gitlab.
