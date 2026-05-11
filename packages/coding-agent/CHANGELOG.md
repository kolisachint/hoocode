# Changelog

## [Unreleased]

### Added

- Added NVIDIA provider support with `NVIDIA_API_KEY` authentication and default model `meta/llama-3.3-70b-instruct`.
- Added mode-based tool filtering via `enabled_tools` configuration option (mode takes priority over profile).
- Added write path restrictions via `allowed_write_paths` configuration option for granular file access control.

### Fixed

- Fixed branding: changed default share viewer URL from `pi.dev` to `hoocode.dev`.
- Fixed branding: changed APP_TITLE from lowercase `hoocode` to `HooCode`.
- Fixed changelog showing old entries on every login by resetting changelog for new project.
