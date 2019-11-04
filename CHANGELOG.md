# Changelog

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