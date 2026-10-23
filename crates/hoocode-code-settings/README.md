# hoocode-code-settings

The global (`~/.hoocode/settings.json`) and project (`.hoocode/settings.json`) settings, their
merge, and read-modify-write persistence that only overwrites the fields changed in this
session.

Settings are kept as the raw JSON object so unknown and future keys pass through untouched;
the manager's getters give typed, defaulted values.
