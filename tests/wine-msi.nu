#!/usr/bin/env nu
# Install the Windows MSI under Wine, use what it installed, and uninstall it.
#
#     xvfb-run -a -s '-screen 0 1280x1024x24' nu tests/wine-msi.nu MSI HOST_DIR COMPONENT REPLAY_DIR OUT_DIR
#
# WINEPREFIX must name an initialized Wine prefix; the MSI installs into its C:
# drive. HOST_DIR holds the portable-host.exe and DLLs that were packaged, and
# COMPONENT the packaged moving-square.wasm. The test
#
# 1. installs the MSI with `msiexec /i` (log: OUT_DIR/install.log);
# 2. checks that exactly the packaged files were installed, byte for byte,
#    that Windows lists the product as installed, and that the Start-menu
#    shortcut exists (OUT_DIR/shortcut.txt is winedump's reading of it);
# 3. replays every script with the installed host and component, which must
#    reproduce every golden trace;
# 4. opens the shortcut the way Wine's own menu entries do (`start /unix`),
#    waits for the game's window (OUT_DIR/shortcut.png is the whole screen),
#    and requires the window to close on Q;
# 5. uninstalls with `msiexec /x` (log: OUT_DIR/uninstall.log) and checks that
#    the files and the shortcut are gone.
#
# Needs wine, winepath, winedump, xdotool, and ImageMagick on PATH. The
# evidence goes to OUT_DIR/report.md.

def fail [message: string] {
    error make --unspanned {msg: $"msi test: ($message)"}
}

# The Windows form of a Unix path, as Wine maps it.
def windows-path [path: path] {
    ^winepath --windows $path | str trim
}

def windows-titled [title: string] {
    ^xdotool search --onlyvisible --name $'^($title)$' | complete | get stdout | lines | where $it != ''
}

# Wait up to `seconds` for a visible window with exactly this title.
def wait-for-window [title: string, seconds: int] {
    for _ in 1..($seconds * 10) {
        if not (windows-titled $title | is-empty) { return }
        sleep 100ms
    }
    fail $"no window titled `($title)` appeared within ($seconds) s"
}

# Give the keyboard to the window with this title. Wine may replace a window's
# X window while it starts, so the window is looked up again until focusing it
# succeeds.
def focus-window [title: string, seconds: int] {
    for _ in 1..($seconds * 10) {
        let ids = (windows-titled $title)
        if not ($ids | is-empty) {
            if (^timeout 10 xdotool windowfocus --sync ($ids | first) | complete).exit_code == 0 { return }
        }
        sleep 100ms
    }
    fail $"cannot focus the window `($title)` within ($seconds) s"
}

# Wait up to `seconds` until no window has this title.
def wait-for-no-window [title: string, seconds: int] {
    for _ in 1..($seconds * 10) {
        if (windows-titled $title | is-empty) { return }
        sleep 100ms
    }
    fail $"the window `($title)` was still open after ($seconds) s"
}

def files-in [dir: path] {
    if not ($dir | path exists) { return [] }
    ls $dir | get name | path basename | sort
}

# Windows Installer logs are UTF-16 on Windows; accept either encoding.
def read-log [path: path] {
    let bytes = (open --raw $path | into binary)
    if ($bytes | bytes index-of 0x[00]) == -1 { $bytes | decode utf-8 } else { $bytes | decode utf-16le }
}

def main [msi: path, host_dir: path, component: path, replay_dir: path, out: path] {
    mkdir $out
    let drive_c = ($env.WINEPREFIX | path join drive_c)
    let install_dir = ($drive_c | path join 'Program Files' 'Portable Host')
    let installed_host = 'C:\Program Files\Portable Host\portable-host.exe'
    let installed_component = 'C:\Program Files\Portable Host\moving-square.wasm'

    # 1. Install quietly, as from an administrator's command prompt.
    let install = (^wine msiexec /i (windows-path $msi) /qn /l*v (windows-path ($out | path join install.log)) | complete)
    if $install.exit_code != 0 { fail $"msiexec /i exited with ($install.exit_code): ($install.stderr)" }

    # 2. Exactly the packaged files, byte for byte, and the uninstall entry.
    let packaged = (ls $host_dir | get name | each {|f| {name: ($f | path basename), source: $f} }
        | append {name: 'moving-square.wasm', source: $component})
    let installed = (files-in $install_dir)
    let expected = ($packaged | get name | append 'LICENSE.txt' | sort)
    if $installed != $expected {
        fail $"installed ($installed | str join ', '), expected ($expected | str join ', ')"
    }
    let file_rows = ($packaged | each {|file|
        let target = ($install_dir | path join $file.name)
        let sha256 = (open --raw $target | hash sha256)
        if $sha256 != (open --raw $file.source | hash sha256) { fail $"the installed ($file.name) differs from the packaged one" }
        {name: $file.name, bytes: (ls $target | get 0.size | into int), sha256: $sha256}
    })
    # `reg query /s` prints one block per key: its path, then its values.
    let entries = (^wine reg query 'HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall' /s | complete)
    let entry = ($entries.stdout | str replace -a "\r" '' | split row "\n\n"
        | where $it =~ '(?m)^\s+DisplayName\s+REG_SZ\s+Portable Host\s*$' | get -o 0)
    if $entry == null { fail $"Windows does not list Portable Host as installed:\n($entries.stdout)" }
    let value = {|name| $entry | parse -r ('(?m)^\s+' + $name + '\s+REG_SZ\s+(?<value>.*)$') | get -o 0.value | default '' | str trim }
    let product_code = ($entry | lines | where $it != '' | first | str trim | split row '\' | last)
    let version = (do $value DisplayVersion)
    let publisher = (do $value Publisher)

    # The Start menu's Programs folder, whichever the installer chose.
    let found = (glob ($drive_c | path join '**' 'Start Menu' Programs 'Portable Host' 'Moving Square.lnk'))
    if ($found | is-empty) {
        let links = (glob ($drive_c | path join '**' '*.lnk') | each { path relative-to $drive_c } | str join "\n")
        let log = (read-log ($out | path join install.log) | lines | where $it =~ '(?i)shortcut' | str join "\n")
        fail $"no Start-menu shortcut Portable Host\\Moving Square.lnk. Shortcuts on C:\n($links)\nThe install log on shortcuts:\n($log)"
    }
    let shortcut = ($found | first)
    let menu_dir = ($shortcut | path dirname)
    let shortcut_location = ('C:\' + ($shortcut | path relative-to $drive_c | str replace -a '/' '\'))
    let link = (^winedump dump $shortcut | complete)
    $link.stdout | save -f ($out | path join shortcut.txt)
    # winedump prints the target as `LocalPath ofs = ... ("C:\...")`.
    let wanted = [
        $"\(\"($installed_host)\"\)"
        $'Arguments : play "($installed_component)"'
        'Working directory : C:\Program Files\Portable Host\'
    ]
    for want in $wanted {
        if not ($link.stdout | str contains $want) { fail $"the shortcut does not hold `($want)`:\n($link.stdout)" }
    }

    # 3. The installed host replays every script with the installed component.
    let scripts = (glob ($replay_dir | path join '*.script') | sort)
    let replays = ($scripts | each {|script|
        let name = ($script | path parse | get stem)
        let golden = ($replay_dir | path join $"($name).trace")
        let result = (^wine $installed_host replay $installed_component $script --expect $golden | complete)
        $result.stdout | save -f ($out | path join $"replay-($name).out")
        if $result.exit_code != 0 { fail $"the installed host failed on ($name):\n($result.stdout)\n($result.stderr)" }
        let trace = ($result.stdout | parse -r 'trace\s+sha256 (?<sha256>[0-9a-f]+) \((?<frames>\d+) frames\)' | get 0)
        {script: $name, frames: $trace.frames, trace_sha256: $trace.sha256}
    })

    # 4. Open the shortcut the way Wine's menu entries do. As from Explorer,
    #    the game gets a console of its own and `start` returns at once, but
    #    the game keeps the output `start` was given, so `start` runs as a job
    #    that ends with the game. Follow the game's window: it must appear, and
    #    close on Q.
    hide-env -i WAYLAND_DISPLAY
    let title = 'moving-square - portable-host'
    let launch_file = ($out | path join shortcut-start.nuon)
    let launcher = job spawn { ^wine start /unix $shortcut | complete | to nuon | save -f $launch_file }
    wait-for-window $title 60
    sleep 2sec
    ^magick import -window root ($out | path join shortcut.png)
    focus-window $title 30
    sleep 300ms
    ^xdotool key q
    wait-for-no-window $title 30
    for _ in 1..300 {
        if not ($launcher in (job list | get id)) { break }
        sleep 100ms
    }
    if ($launcher in (job list | get id)) { fail "the game was still running 30 s after its window closed" }
    let launch = (open $launch_file)
    if $launch.exit_code != 0 { fail $"start exited with ($launch.exit_code): ($launch.stderr)" }

    # 5. Uninstall, and nothing of it is left.
    let uninstall = (^wine msiexec /x (windows-path $msi) /qn /l*v (windows-path ($out | path join uninstall.log)) | complete)
    if $uninstall.exit_code != 0 { fail $"msiexec /x exited with ($uninstall.exit_code): ($uninstall.stderr)" }
    let left = (files-in $install_dir | append (files-in $menu_dir))
    if not ($left | is-empty) { fail $"uninstalling left ($left | str join ', ')" }

    let report = ([
        "# Windows installer report (under Wine)"
        ""
        $"Installer: `($msi | path basename)`, installed with `msiexec /i /qn` and removed with `msiexec /x /qn` under (^wine --version | str trim)."
        ""
        $"Installed to `C:\\Program Files\\Portable Host`, each file identical to the one packaged. Windows lists \"Portable Host\" ($version) by ($publisher) as installed, product code `($product_code)`."
        ""
        "| File | Bytes | sha256 |"
        "| --- | --- | --- |"
        ...($file_rows | each {|f| $"| ($f.name) | ($f.bytes) | `($f.sha256)` |" })
        ""
        $"Start-menu shortcut `($shortcut_location)` runs `($installed_host)` with `play \"($installed_component)\"`. Opened with `start /unix`, it showed the window `($title)` \(shortcut.png\), which closed on Q."
        ""
        "The installed host, replaying with the installed component:"
        ""
        "| Script | Frames | Trace sha256 | Result |"
        "| --- | --- | --- | --- |"
        ...($replays | each {|r| $"| ($r.script) | ($r.frames) | `($r.trace_sha256)` | identical to the golden trace |" })
        ""
        "Uninstalling removed every installed file and the shortcut."
        ""
    ] | str join "\n")
    $report | save -f ($out | path join report.md)
    print $report
}
