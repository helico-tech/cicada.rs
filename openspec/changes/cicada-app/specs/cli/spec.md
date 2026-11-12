# cli

## ADDED Requirements

### Requirement: The terminal UI is a feature of the build

The command `tui` SHALL exist only in a binary built with the feature `tui` of `cicada-app`, which is off by
default. A binary built without it SHALL NOT depend on the libraries of the terminal UI.

#### Scenario: Built without the feature

- **WHEN** a binary built without the feature is run with `tui`
- **THEN** the command line refuses the command as one it does not know

#### Scenario: Built with the feature

- **WHEN** a binary built with the feature is run with `tui`
- **THEN** the terminal UI starts

## MODIFIED Requirements

### Requirement: The CLI reads its config file from a fixed path

The config file SHALL be `$CICADA_CONFIG`, else `$XDG_CONFIG_HOME/cicada/config.toml`, else
`$HOME/.config/cicada/config.toml`. A missing file SHALL count as an empty one. On Unix, a command that writes the
file SHALL leave it with mode 0600. A command that writes a path that is a symlink SHALL write the file the link
points to and leave the link as it is.

#### Scenario: XDG config home

- **WHEN** `CICADA_CONFIG` is unset and `XDG_CONFIG_HOME` is `/x`
- **THEN** the file read is `/x/cicada/config.toml`

#### Scenario: The file stays private

- **WHEN** on Unix `cicada config set-context` writes the file, which existed with mode 0644
- **THEN** the file has mode 0600 afterwards, and a reader that opened the old file before reads only the old text

#### Scenario: A path that is a symlink

- **WHEN** the config path is a symlink to a file elsewhere and `cicada config set-context` writes it
- **THEN** the path is still a symlink, and the file it points to holds the new text with mode 0600

#### Scenario: A relative link, and one whose target is not there

- **WHEN** the config path is a symlink with a relative target that does not exist yet, and `cicada config
  set-context` writes it
- **THEN** the target is created in the folder the link names, relative to the link, and the path is still a
  symlink
