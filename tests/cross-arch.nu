#!/usr/bin/env nu
# Run every replay script on several hosts and require byte-identical traces.
#
#     nu tests/cross-arch.nu COMPONENT REPLAY_DIR OUT_DIR HOST...
#
# Each HOST is `label=command`, where the command runs a portable-host binary,
# for example `x86_64=result/bin/portable-host`,
# `aarch64=qemu-aarch64 result-aarch64/bin/portable-host`, or
# `windows=wine result-windows/bin/portable-host.exe`. Every host must
# reproduce every golden trace exactly. The evidence goes to OUT_DIR/report.md.

def fail [message: string] {
    error make --unspanned {msg: $"cross-arch: ($message)"}
}

def main [component: path, replay_dir: path, out: path, ...hosts: string] {
    if ($hosts | length) < 2 { fail "give at least two hosts to compare" }
    mkdir $out
    let hosts = ($hosts | each {|spec|
        let parts = ($spec | split row -n 2 '=')
        if ($parts | length) != 2 { fail $"`($spec)` is not label=command" }
        {label: $parts.0, command: ($parts.1 | split row ' ' | where $it != '')}
    })
    let component_sha256 = (open --raw $component | hash sha256)
    let scripts = (glob ($replay_dir | path join '*.script') | sort)
    if ($scripts | is-empty) { fail $"no scripts in ($replay_dir)" }

    let runs = ($scripts | each {|script|
        let name = ($script | path parse | get stem)
        let golden = ($replay_dir | path join $"($name).trace")
        $hosts | each {|host|
            let trace = ($out | path join $"($host.label)-($name).trace")
            let result = (^($host.command.0) ...($host.command | skip 1) replay $component $script --trace $trace --expect $golden | complete)
            $result.stdout | save -f ($out | path join $"($host.label)-($name).out")
            if $result.exit_code != 0 {
                fail $"($host.label) failed on ($name):\n($result.stdout)\n($result.stderr)"
            }
            let field = {|key| $result.stdout | lines | where $it starts-with $key | first | str replace -r $'^($key)\s+' '' }
            {
                script: $name
                host: $host.label
                runs_as: (do $field 'host')
                component_sha256: ($result.stdout | lines | where $it =~ '^\s+sha256' | first | str trim | str replace -r '^sha256\s+' '')
                ticks: ($result.stdout | parse -r 'script\s+.*, (?<ticks>\d+) ticks' | get 0.ticks)
                trace_sha256: (open --raw $trace | hash sha256)
            }
        }
    } | flatten)

    for run in $runs {
        if $run.component_sha256 != $component_sha256 {
            fail $"($run.host) ran a component with sha256 ($run.component_sha256)"
        }
    }
    let per_script = ($runs | group-by script --to-table | each {|group|
        let hashes = ($group.items | get trace_sha256 | uniq)
        if ($hashes | length) != 1 { fail $"hosts disagree on ($group.script): ($group.items | select host trace_sha256)" }
        {script: $group.script, ticks: $group.items.0.ticks, trace_sha256: $hashes.0}
    })

    let host_rows = ($hosts | each {|host|
        let runs_as = ($runs | where host == $host.label | first | get runs_as)
        let program = ($host.command.0 | path basename)
        let executed = if $program =~ '^qemu-' {
            'QEMU user-mode emulation'
        } else if $program =~ '^wine' {
            'Wine (the Windows API on Linux), not Windows'
        } else {
            'natively'
        }
        $"| ($host.label) | `($runs_as)` | ($executed) | `($host.command | str join ' ')` |"
    })
    let script_rows = ($per_script | each {|row|
        $"| ($row.script) | ($row.ticks) | `($row.trace_sha256)` | identical on ($hosts | length) hosts and to the golden trace |"
    })
    let report = ([
        "# Cross-platform replay report"
        ""
        $"Component: `($component)`"
        ""
        $"Component sha256: `($component_sha256)`; every host reported this same hash."
        ""
        "| Host | Binary reports | Executed | Command |"
        "| --- | --- | --- | --- |"
        ...$host_rows
        ""
        "| Script | Ticks | Trace sha256 | Result |"
        "| --- | --- | --- | --- |"
        ...$script_rows
        ""
    ] | str join "\n")
    $report | save -f ($out | path join report.md)
    print $report
}
