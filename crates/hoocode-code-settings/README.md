# hoocode-code-settings

Port of hoocode `core/settings-{types,defaults,storage,manager}.ts` (v0.5.89): the global
(`~/.hoocode/settings.json`) and project (`.hoocode/settings.json`) settings, their
merge, and read-modify-write persistence that only overwrites the fields changed in this
session.

Settings are kept as the raw JSON object so unknown and future keys pass through untouched;
the manager's getters give typed, defaulted values. hoocode's `.hoocode/settings.json` is read
when the hoocode file does not exist yet, and the first write creates the hoocode file.
