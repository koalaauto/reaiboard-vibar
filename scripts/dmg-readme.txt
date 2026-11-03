Vibar @VERSION@  (internal build)
=================================

1. Drag Vibar onto Applications.

2. This build is not notarized by Apple, so macOS may refuse the first launch.
   Run this once in Terminal, then open Vibar:

       xattr -dr com.apple.quarantine /Applications/Vibar.app

   Or open it, then System Settings > Privacy & Security > Open Anyway.

3. Allow Bluetooth when macOS asks. Allow Accessibility when the menu shows
   "Accessibility permission needed...". After each update macOS asks again,
   that is expected for this kind of build.

4. Unplug the USB cable and press any key on the keyboard.
   Do NOT pair it in System Settings > Bluetooth.

5. The keyboard connects to one computer at a time. If it is connected to
   another computer, disconnect it there first (menu > Disconnect Keyboard,
   or quit Vibar), then press any key on the keyboard.

Needs a Mac with Apple silicon, macOS 12 or later.


Vibar @VERSION@  (内部版本)
=================================

1. 把 Vibar 拖进「应用程序」。

2. 这个版本没有经过 Apple 公证,macOS 可能拒绝第一次打开。
   在终端里执行一次下面的命令,再打开 Vibar:

       xattr -dr com.apple.quarantine /Applications/Vibar.app

   或者先打开一次,再到「系统设置 > 隐私与安全性」里点「仍要打开」。

3. macOS 询问时允许蓝牙。菜单里出现「需要辅助功能权限…」时允许辅助功能。
   每次升级后系统会再问一遍,这类版本就是这样,属于正常现象。

4. 拔掉 USB 线,按键盘上任意键。
   不要在「系统设置 > 蓝牙」里配对这块键盘。

5. 键盘同一时间只连一台电脑。如果它连着另一台电脑,请先在那台电脑上断开
   (菜单里点「断开键盘」,或者退出 Vibar),再按键盘上任意键。

需要 Apple 芯片的 Mac,macOS 12 或更高版本。
