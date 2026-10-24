#!/usr/bin/env nu
# End-to-end check of the graphical backend's keyboard input.
#
# Run it inside a virtual X server so it never touches a real desktop session:
#
#     xvfb-run -a nu tests/keyboard.nu PORTABLE_HOST COMPONENT OUT_DIR
#
# PORTABLE_HOST is the command that runs portable-host, for example
# `target/release/portable-host` or `wine result/bin/portable-host.exe`.
#
# It plays the game with real X key events from xdotool: hold Right, then
# Down; pause with P; hold Left while stepping three ticks with N; resume with
# Esc; quit with Q. It then checks the input the host recorded and that the
# host's own headless cross-check passed.

def fail [message: string] {
    error make --unspanned {msg: $"keyboard test: ($message)"}
}

# Give the keyboard to the visible window with this exact title, waiting up to
# `seconds` for it. Wine may replace a window's X window while it starts, so
# the window is looked up again until focusing it succeeds.
def focus-window [title: string, seconds: int] {
    for _ in 1..($seconds * 10) {
        let ids = (^xdotool search --onlyvisible --name $'^($title)$' | complete | get stdout | lines | where $it != '')
        if not ($ids | is-empty) {
            if (^timeout 10 xdotool windowfocus --sync ($ids | first) | complete).exit_code == 0 { return }
        }
        sleep 100ms
    }
    fail $"cannot focus a window titled `($title)` within ($seconds) s"
}

def main [host: string, component: path, out: path] {
    mkdir $out
    let command = ($host | split row ' ' | where $it != '')
    let record = ($out | path join keyboard.script)
    let shot = ($out | path join keyboard.png)
    let result = ($out | path join play.nuon)
    rm -f $record $shot $result

    let game = job spawn {
        # Leave SDL (or Wine) nothing to choose but the virtual X server.
        hide-env -i WAYLAND_DISPLAY SDL_VIDEO_DRIVER
        ^($command.0) ...($command | skip 1) play $component --record $record --screenshot $shot
        | complete | to nuon | save -f $result
    }

    focus-window 'moving-square - portable-host' 60
    sleep 500ms

    ^xdotool keydown Right
    sleep 1sec
    ^xdotool keyup Right
    ^xdotool keydown Down
    sleep 500ms
    ^xdotool keyup Down
    ^xdotool key p
    sleep 300ms
    ^xdotool keydown Left
    sleep 100ms
    ^xdotool key n n n
    sleep 300ms
    ^xdotool keyup Left
    ^xdotool key Escape
    sleep 300ms
    ^xdotool key q

    mut waited = 0
    while ($game in (job list | get id)) {
        if $waited > 200 { fail "the game did not exit after Q" }
        sleep 100ms
        $waited += 1
    }
    if not ($result | path exists) { fail "the game produced no result" }
    let play = (open $result)
    if $play.exit_code != 0 { fail $"portable-host failed:\n($play.stdout)\n($play.stderr)" }
    if not ($play.stdout | str contains 'headless   replaying the same') {
        fail $"the headless cross-check did not pass:\n($play.stdout)"
    }
    if not ($shot | path exists) { fail "no screenshot was saved" }

    # Turn the recorded script into runs of identical input.
    let lines = (open --raw $record | lines | where $it != '')
    let total = ($lines | first | parse 'ticks {n}' | get 0.n | into int)
    let entries = ($lines | skip 1 | parse '{tick} {held}' | update tick { into int })
    let runs = ($entries | enumerate | each {|e|
        let next = ($entries | get -o ($e.index + 1) | get -o tick | default $total)
        {held: $e.item.held, ticks: ($next - $e.item.tick)}
    })
    let ticks_of = {|held| $runs | where held == $held | get ticks | math sum }
    let position = {|held| $runs | enumerate | where item.held == $held | get -o 0.index }

    let right = (do $ticks_of 'right')
    let down = (do $ticks_of 'down')
    let left_runs = ($runs | where held == 'left')
    if not ($right in 30..90) { fail $"expected about 60 ticks of right, got ($right)" }
    if not ($down in 10..45) { fail $"expected about 30 ticks of down, got ($down)" }
    if ($left_runs | length) != 1 or ($left_runs.0.ticks != 3) {
        fail $"expected exactly 3 ticks of left, stepped while paused; got ($left_runs)"
    }
    if not ((do $position 'right') < (do $position 'down') and (do $position 'down') < (do $position 'left')) {
        fail $"inputs were recorded out of order: ($runs)"
    }
    let after = ($runs | last)
    if $after.held != '-' or $after.ticks < 1 {
        fail $"the game did not resume after Esc: ($runs)"
    }
    if not ($play.stdout | str contains $"played     ($total) ticks") {
        fail $"the recording has ($total) ticks but the host reported otherwise:\n($play.stdout)"
    }

    print $"keyboard ok: right ($right), down ($down), left 3 while paused, ($after.ticks) idle after resuming; ($total) ticks in all"
    print ($play.stdout | lines | where $it =~ '^(display|played|headless)' | str join "\n")
}
