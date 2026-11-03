# Design principles

Rules for anything a user sees. They exist because every one of them was broken once and a person noticed.

## Product

1. **Few things in the menu.** The menu bar item shows status, one frequent action (disconnect or connect the keyboard), *Settings…*, *About* and *Quit*. Everything else lives in the settings window.
2. **Normal things stay quiet.** No row says "permission granted". A permission row appears only when a permission is missing, and disappears when it is granted.
3. **Every input gets feedback.** If the keyboard sends something and nothing on screen changes, the user concludes it is broken. The voice key shows a yellow dot on the menu bar icon and plays a short sound. In the settings window a pressed input lights up.
4. **Nothing moves.** Icons and menu rows change content, never size. The voice-key icon is a different picture on an identical canvas so the menu bar does not shift.
5. **Never write to the keyboard silently.** The only write is the sleep time, and only when the user picks a value. Skip writes that would not change anything.
6. **Disabled controls explain themselves.** A greyed control carries a line saying what enables it, for example "连接键盘后才能修改".
7. **Never persist a half-finished edit.** Choosing "shortcut" without recording one must not write an empty shortcut. Keep it as a draft in the page.
8. **Errors appear where the user is looking.** An error caused by a click in the settings window is shown in the settings window, not only in the menu.
9. **No test buttons for users.** If a feature needs a "test" button to be trusted, show its real state instead (for example the microphone row says whether an app is recording).

## Copy

The interface language is Chinese today. These rules are about style and apply to any language.

- **No parentheses and no colons** to add explanations or to attach a value. Write a sentence, or split into two lines. Not "释放键盘(借给别的电脑)", not "麦克风:空闲".
- **Short verb phrases for actions**: "断开键盘", "连接键盘", "卸载". Do not explain the action inside its label.
- **No jargon the user did not bring.** Not "驱动", "校准", "输入", "绑定" when a plainer word works. Hardware labels (Tab, New, Esc, Action, Enter) are the keyboard's own names and stay.
- **Do not editorialize defaults.** No "(默认)" next to an option. Show the current value with a tick.
- **A button says what it does**, an empty field says what to do ("点击后按下新的快捷键").
- **State in plain words**: "已连接", "正在搜索键盘…", "电量 94%". No device ids, no internal state names.
- Use `…` only for something that is in progress or that opens another window.

`settings::tests::page_text_avoids_parentheses_and_colons_in_visible_copy` enforces the punctuation rule for the settings page. Menu strings in `ui.rs` are checked by review.

## Icons

All icons are produced by `scripts/make-icons.py` from one geometry definition. Change the script and regenerate, never edit the PNGs by hand.

- **Menu bar**: a control surface with a knob on the left and six keys on the right. Canvas 46×36 px (23×18 pt at 2x), body 34×22 with generous margin so the system highlight does not touch it.
  - Connected: filled body, knob and keys cut out.
  - Disconnected: outline only. State is carried by shape, not only by brightness or colour.
  - Voice key held: same filled glyph with a yellow dot hanging off the top right corner. It is a colour image, so it is not a template image. White or black body is chosen by asking the status item button for its real appearance, because the menu bar colour follows the wallpaper and can differ from the system light or dark setting.
- **Knob**: always a complete circle. No notch, no gap.
- **App icon**: dark rounded square, light control surface, one blue dot on the knob. Check it at 16 px.
- Rendering uses signed distance functions with 8×8 supersampling. Never draw icons with hard pixels.

## Settings window

- Sidebar of four pages with icons, status card and version at the bottom.
- **Autosave.** No Save button. A small "已保存" fades in near the layer tabs.
- A layer other than "所有档位" shows inherited keys dimmed and overridden keys with a blue dot, with a *跟随所有档位* button to drop an override.
- Both light and dark mode are supported. Look at both before you finish.
- Window screenshots for reviews: capture the window by its window id, never a screen region. A region can contain whatever is in front of the window, including private content.
