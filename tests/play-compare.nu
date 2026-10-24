#!/usr/bin/env nu
# Play one script in the graphical backend of several hosts and compare what
# they drew.
#
#     xvfb-run -a -s '-screen 0 1280x1024x24' nu tests/play-compare.nu COMPONENT SCRIPT OUT_DIR HOST...
#
# Each HOST is `label=command`, as for tests/cross-arch.nu, for example
# `linux=result/bin/portable-host` or
# `windows=wine result-windows/bin/portable-host.exe`. Every host plays SCRIPT
# in a window and quits at its end. Its last frame must be the one the golden
# trace (SCRIPT with the extension .trace) ends on, and its screenshot,
# OUT_DIR/LABEL.png, must match the first host's pixel for pixel. Needs
# ImageMagick. The evidence goes to OUT_DIR/report.md.

def fail [message: string] {
    error make --unspanned {msg: $"play-compare: ($message)"}
}

def main [component: path, script: path, out: path, ...hosts: string] {
    if ($hosts | is-empty) { fail "give at least one host" }
    mkdir $out
    let golden = ($script | path parse | update extension trace | path join)
    let last_frame = (open --raw $golden | lines | where $it != '' | last | split row -n 2 ' | ' | get 1)
    let expected = ($last_frame | hash sha256)
    let ticks = (open --raw $script | lines | parse -r '^\s*ticks\s+(?<n>\d+)' | get 0.n | into int)
    # Draw on the private X server only, never on a Wayland session.
    hide-env -i WAYLAND_DISPLAY

    let runs = ($hosts | each {|spec|
        let parts = ($spec | split row -n 2 '=')
        if ($parts | length) != 2 { fail $"`($spec)` is not label=command" }
        let label = $parts.0
        let command = ($parts.1 | split row ' ' | where $it != '')
        let shot = ($out | path join $"($label).png")
        let result = (^($command.0) ...($command | skip 1) play $component --script $script --exit-at-end --screenshot $shot | complete)
        $result.stdout | save -f ($out | path join $"($label).out")
        if $result.exit_code != 0 { fail $"($label) failed:\n($result.stdout)\n($result.stderr)" }
        let field = {|key| $result.stdout | lines | where $it starts-with $key | first | str replace -r $'^($key)\s+' '' }
        let played = (do $field 'played' | parse -r '^(?<ticks>\d+) ticks; final frame sha256 (?<sha256>[0-9a-f]{64})$')
        if ($played | is-empty) { fail $"($label) did not report the frame it ended on:\n($result.stdout)" }
        if ($played.0.ticks | into int) != $ticks { fail $"($label) played ($played.0.ticks) ticks, not ($ticks)" }
        if $played.0.sha256 != $expected {
            fail $"($label) ended on frame ($played.0.sha256), not on the golden trace's last frame ($expected)"
        }
        if not ($result.stdout | str contains 'headless   replaying the same') {
            fail $"($label): the headless cross-check did not pass:\n($result.stdout)"
        }
        if not ($shot | path exists) { fail $"($label) saved no screenshot" }
        {
            label: $label
            command: ($command | str join ' ')
            runs_as: (do $field 'host')
            display: (do $field 'display')
            size: (^magick identify -format '%wx%h' $shot)
            png: $shot
        }
    })

    let reference = ($runs | first)
    let rows = ($runs | each {|run|
        let differing = if $run.label == $reference.label { 0 } else {
            # AE prints the number of differing pixels on stderr; exit code 2
            # means the images could not be compared (different sizes, say).
            let compared = (^magick compare -metric AE $reference.png $run.png null: | complete)
            if $compared.exit_code == 2 {
                fail $"cannot compare ($run.label) with ($reference.label): ($compared.stderr)"
            }
            $compared.stderr | str trim | split row ' ' | first | into float | into int
        }
        $run | insert differing_pixels $differing
    })

    let host_rows = ($rows | each {|row|
        let match = if $row.label == $reference.label { 'reference' } else if $row.differing_pixels == 0 {
            $"identical to ($reference.label)"
        } else {
            $"($row.differing_pixels) pixels differ from ($reference.label)"
        }
        $"| ($row.label) | `($row.runs_as)` | ($row.display) | ($row.size) | ($match) |"
    })
    let report = ([
        "# Graphical replay report"
        ""
        $"Component: `($component)`"
        ""
        $"Script: `($script | path basename)`, ($ticks) ticks. Every host ended on the golden trace's last frame, sha256 `($expected)`, and its own headless replay of the session ended there too."
        ""
        "| Host | Binary reports | Display | Screenshot | Pixels |"
        "| --- | --- | --- | --- | --- |"
        ...$host_rows
        ""
    ] | str join "\n")
    $report | save -f ($out | path join report.md)
    print $report

    let mismatched = ($rows | where differing_pixels != 0)
    if not ($mismatched | is-empty) {
        fail $"screenshots differ from ($reference.label): ($mismatched | select label differing_pixels)"
    }
}
