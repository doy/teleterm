# Changelog

## Unreleased

### Added

* `tt play` now supports hiding the pause ui.

### Fixed

* Bump `vt100` dep to fix a bunch of parsing bugs.

## [0.1.6] - 2019-11-07

### Added

* `tt play` now has key commands for seeking to the start or end of the file.
* `tt play` now allows searching for frames whose contents match a regex.

## [0.1.5] - 2019-11-06

### Fixed

* Fix clearing the screen in `tt watch`.

## [0.1.4] - 2019-11-06

### Added

* `tt play` now supports seeking back and forth as well as pausing, adjusting
  the playback speed, and limiting the max amount of time each frame can take.

### Changed

* Moved quite a lot of functionality out to separate crates - see
  `component-future`, `tokio-pty-process-stream`, `tokio-terminal-resize`,
  `ttyrec`

### Fixed

* Ttyrecs with frame timestamps not starting at 0 can now be played properly.

## [0.1.3] - 2019-10-23

### Fixed

* if a system user defines a home directory of `/`, treat it as not having a
  home directory

## [0.1.2] - 2019-10-23

### Fixed

* set both the real and effective uid and gid instead of just effective when
  dropping privileges

## [0.1.1] - 2019-10-23

### Fixed

* wait to drop privileges (via the `uid` and `gid` options until after we have
  read the `tls_identity_file`)

## [0.1.0] - 2019-10-23

### Added

* Initial release
