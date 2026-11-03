# Configuration

The settings window reads and writes one file: `~/.config/vibar/config.toml`. Saving the file, from the window or by hand, takes effect within about a second. A broken file keeps the previous settings and the menu says which line is wrong. A missing file is created with the defaults.

Old installs that used the name `reai-bar` are migrated from `~/.config/reai-bar/` on first start.

## Inputs

| Name | Physical input |
|---|---|
| `knob_ccw`, `knob_cw` | knob turned counter-clockwise, clockwise |
| `knob_press` | knob pressed |
| `tab`, `new`, `esc`, `voice`, `action`, `enter` | the six keys, named after their labels |

Unknown names are reported in the menu instead of being ignored silently.

## Actions

A binding is either a string or a table.

```toml
tab = "tab"                                   # press a shortcut once
new = "cmd+shift+n"
voice = { hold = "f18" }                      # hold while the key is down
knob_cw = { media = "volume_up" }             # media key
action = { shell = "open -a Safari" }         # run a command
esc = { none = true }                         # do nothing (useful in a layer)
```

| Form | Meaning |
|---|---|
| `"cmd+shift+4"` | click a shortcut. Modifiers: `cmd` `ctrl` `alt` (or `option`) `shift` |
| `{ hold = "f18" }` | press when the key goes down, release when it comes up |
| `{ media = "..." }` | `volume_up`, `volume_down`, `mute`, `play_pause` |
| `{ shell = "..." }` | run with `/bin/sh -c`. It runs as you; only put commands there that you trust. |
| `{ none = true }` | no action |

Keys that can follow the modifiers: single characters, `tab`, `return` (or `enter`), `esc`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1` to `f20`.

## Layers

The lever has three positions: `chat`, `yolo`, `plan`. `[bindings]` is the default layer and applies in every position. A `[modes.<name>]` table overrides single inputs for that position only.

```toml
[bindings]
enter = "return"
tab = "tab"

[modes.plan]
on_enter = "shift+tab"          # runs when the lever arrives at PLAN
on_leave = { shell = "say bye" } # runs when it leaves

[modes.plan.bindings]
enter = "cmd+return"            # only in PLAN, other keys keep the default
```

Lookup order for a key: the current position's layer, then `[bindings]`. The binding chosen when a key goes down is the one used when it comes up.

On connect, the first position report only records where the lever is. `on_enter` and `on_leave` run only when the lever actually moves.

## Other keys

| Key | Meaning |
|---|---|
| `device = "REAI_VB_xxxxxxxx"` | connect to this keyboard instead of the first one found. Useful with several boards. |
| `knob_swap = true` | swap the two knob directions. The settings window can set this by calibration. |

## Editing tips

- The settings window rewrites the file in a canonical form, so comments you add by hand are not kept. Keep notes elsewhere.
- Keep a copy before experimenting with `shell` bindings.
- The shipped defaults live in `default-config.toml` and are checked by unit tests.
