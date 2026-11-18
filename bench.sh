#!/usr/bin/env bash
set -euo pipefail

INTERVAL=5
COUNT=0
OUT=""
PID=""
PYTHON=0

usage() {
    cat <<EOF
Usage: ./bench.sh [options]

Measures memory and CPU of a running terminalchat (Rust version by default)
and prints one line per sample, each starting with date and time. CPU is the
load during the last interval, not the average since start. At the end
(ctrl+c, --count or when the program exits) a summary is printed.

Options:
  -i, --interval SEC   seconds between samples (default 5)
  -n, --count N        stop after N samples (default: run until stopped)
  -o, --output FILE    also append the lines to FILE
  -p, --pid PID        measure this process
  --python             measure the Python version (terminalchat.py)
  -h, --help           show this help

Examples:
  ./bench.sh                       watch the Rust client
  ./bench.sh -i 1 -n 60 -o rust.txt
  ./bench.sh --python -o python.txt
EOF
}

fail() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -i|--interval) INTERVAL="${2:?missing value}"; shift ;;
            -n|--count) COUNT="${2:?missing value}"; shift ;;
            -o|--output) OUT="${2:?missing value}"; shift ;;
            -p|--pid) PID="${2:?missing value}"; shift ;;
            --python) PYTHON=1 ;;
            -h|--help) usage; exit 0 ;;
            *) usage >&2; fail "unknown option: $1" ;;
        esac
        shift
    done
    [[ "${INTERVAL}" =~ ^[0-9]+$ && "${INTERVAL}" -ge 1 ]] || fail "interval must be a whole number of seconds"
    [[ "${COUNT}" =~ ^[0-9]+$ ]] || fail "count must be a number"
}

find_pid() {
    local dir pid exe cmdline
    for dir in /proc/[0-9]*; do
        pid="${dir#/proc/}"
        [[ "${pid}" == "$$" ]] && continue
        exe="$(readlink "${dir}/exe" 2>/dev/null || true)"
        [[ -n "${exe}" ]] || continue
        if [[ "${PYTHON}" -eq 1 ]]; then
            cmdline="$(tr '\0' ' ' < "${dir}/cmdline" 2>/dev/null || true)"
            if [[ "${exe}" == *python* && "${cmdline}" == *terminalchat.py* ]]; then
                echo "${pid}"
                return 0
            fi
        else
            case "${exe}" in
                */terminalchat|*/terminalchat-*) echo "${pid}"; return 0 ;;
            esac
        fi
    done
    return 1
}

cpu_ticks() {
    # utime + stime; the part up to the last ")" is the command name
    sed 's/.*) //' "/proc/$1/stat" | awk '{print $12 + $13}'
}

status_kb() {
    awk -v key="$2:" '$1 == key {print $2}' "/proc/$1/status"
}

emit() {
    if [[ -n "${OUT}" ]]; then
        printf '%s\n' "$1" | tee -a "${OUT}"
    else
        printf '%s\n' "$1"
    fi
}

summary() {
    [[ -s "${SAMPLES}" ]] || return 0
    awk '
        { n++; rss += $1; if (n == 1 || $1 < rmin) rmin = $1; if ($1 > rmax) rmax = $1;
          cpu += $2; if ($2 > cmax) cmax = $2 }
        END {
            printf "samples %d | RSS min %.1f avg %.1f max %.1f MB | CPU avg %.2f%% max %.2f%%\n",
                n, rmin / 1024, rss / n / 1024, rmax / 1024, cpu / n, cmax
        }' "${SAMPLES}"
}

main() {
    parse_args "$@"
    if [[ -z "${PID}" ]]; then
        PID="$(find_pid)" || fail "no running terminalchat found (use -p PID or start it first)"
    fi
    [[ -d "/proc/${PID}" ]] || fail "process ${PID} does not exist"

    SAMPLES="$(mktemp)"
    trap 'summary_line="$(summary)"; [[ -n "${summary_line}" ]] && emit "${summary_line}"; rm -f "${SAMPLES}"; exit 0' INT TERM

    local clk threads name samples=0 ticks last_ticks now last_time rss peak line stamp cpu
    clk="$(getconf CLK_TCK)"
    name="$(tr '\0' ' ' < "/proc/${PID}/cmdline" | cut -c1-80)"
    emit "$(date '+%Y-%m-%d %H:%M:%S')  measuring PID ${PID}: ${name}"
    last_ticks="$(cpu_ticks "${PID}")"
    last_time="$(date +%s.%N)"

    while [[ -d "/proc/${PID}" ]]; do
        sleep "${INTERVAL}"
        [[ -d "/proc/${PID}" ]] || break
        ticks="$(cpu_ticks "${PID}")"
        now="$(date +%s.%N)"
        rss="$(status_kb "${PID}" VmRSS)"
        peak="$(status_kb "${PID}" VmHWM)"
        threads="$(status_kb "${PID}" Threads)"
        cpu="$(awk -v t="${ticks}" -v l="${last_ticks}" -v n="${now}" -v p="${last_time}" -v c="${clk}" \
            'BEGIN { printf "%.2f", (t - l) / c / (n - p) * 100 }')"
        last_ticks="${ticks}"
        last_time="${now}"
        stamp="$(date '+%Y-%m-%d %H:%M:%S')"
        printf '%s %s\n' "${rss}" "${cpu}" >> "${SAMPLES}"
        line="$(printf '%s  RSS %7.1f MB  peak %7.1f MB  CPU %6s%%  threads %s' \
            "${stamp}" "$(awk -v k="${rss}" 'BEGIN {print k / 1024}')" "$(awk -v k="${peak}" 'BEGIN {print k / 1024}')" "${cpu}" "${threads}")"
        emit "${line}"
        samples=$((samples + 1))
        if [[ "${COUNT}" -gt 0 && "${samples}" -ge "${COUNT}" ]]; then
            break
        fi
    done

    local final
    final="$(summary)"
    [[ -n "${final}" ]] && emit "${final}"
    rm -f "${SAMPLES}"
    [[ -d "/proc/${PID}" ]] || emit "$(date '+%Y-%m-%d %H:%M:%S')  process ${PID} ended"
}

main "$@"
