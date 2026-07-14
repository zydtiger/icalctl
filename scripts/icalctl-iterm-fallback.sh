#!/bin/zsh

set -u
set -o pipefail

readonly DELEGATION_FAILURE=125
readonly USAGE_FAILURE=64
readonly ITERM_BUNDLE_IDENTIFIER="com.googlecode.iterm2"

typeset osascript_bin="/usr/bin/osascript"
typeset delegated_window_id=""
typeset temporary_directory=""
typeset window_id_file=""
typeset tty_file=""
typeset gate_file=""
typeset cancel_file=""
typeset create_focus_file=""
typeset close_focus_file=""
typeset window_closed_file=""
typeset automation_stdout=""
typeset automation_stderr=""
typeset close_stdout=""
typeset close_stderr=""
typeset ownership_token=""
integer interrupted=0
integer active_pid=0
integer timeout_seconds=60
integer operation_deadline=0
integer cleanup_deadline=0
integer close_attempts=0

print_usage() {
    print -u2 -- "usage: ${0:t} ICALCTL_EXECUTABLE [ARG ...]"
}

delegation_failure() {
    print -u2 -- "icalctl iTerm delegation failed: $1"
    exit "$DELEGATION_FAILURE"
}

handle_signal() {
    interrupted=1
}

terminate_process() {
    local pid=$1

    kill -TERM "$pid" >/dev/null 2>&1 || true
    sleep 0.2
    kill -KILL "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
}

run_bounded() {
    integer deadline=$1
    local captured_stdout=$2
    local captured_stderr=$3
    shift 3

    : >"$captured_stdout" || return 126
    : >"$captured_stderr" || return 126
    if (( SECONDS >= deadline )); then
        return 124
    fi

    "$@" >"$captured_stdout" 2>"$captured_stderr" &
    active_pid=$!
    while kill -0 "$active_pid" >/dev/null 2>&1; do
        if (( interrupted || SECONDS >= deadline )); then
            terminate_process "$active_pid"
            active_pid=0
            return 124
        fi
        sleep 0.1
    done

    wait "$active_pid"
    integer child_status=$?
    active_pid=0
    return "$child_status"
}

restore_frontmost_application() {
    local focus_file=$1
    integer deadline=$2

    if [[ ! -f "$focus_file" ]]; then
        return 1
    fi
    typeset bundle_identifier
    bundle_identifier=$(<"$focus_file")
    if [[ ! "$bundle_identifier" =~ '^[A-Za-z0-9.-]+$' ]]; then
        return 1
    fi

    run_bounded "$deadline" "$automation_stdout" "$automation_stderr" \
        "$osascript_bin" - "$bundle_identifier" "$ITERM_BUNDLE_IDENTIFIER" <<'RESTORE_APPLESCRIPT'
use framework "AppKit"
use scripting additions

on frontmostApplication()
    return current application's NSWorkspace's sharedWorkspace()'s frontmostApplication()
end frontmostApplication

on run argv
    set targetBundleIdentifier to item 1 of argv
    set iTermBundleIdentifier to item 2 of argv
    repeat 10 times
        set activeApplication to my frontmostApplication()
        set activeBundleIdentifier to (activeApplication's bundleIdentifier()) as text
        if activeBundleIdentifier is targetBundleIdentifier then return

        -- Do not undo a focus change made by the user while automation was running.
        if activeBundleIdentifier is not iTermBundleIdentifier then return

        set matchingApplications to current application's NSRunningApplication's runningApplicationsWithBundleIdentifier:targetBundleIdentifier
        if (matchingApplications's |count|() as integer) is 0 then error "the previous frontmost application is no longer running"
        set targetApplication to matchingApplications's firstObject()
        set restored to targetApplication's activateWithOptions:(current application's NSApplicationActivateIgnoringOtherApps)
        if restored is false then error "could not restore the frontmost application"
        delay 0.05
    end repeat
    error "frontmost application was not restored"
end run
RESTORE_APPLESCRIPT
}

recover_delegated_window_id() {
    if [[ -n "$delegated_window_id" ]]; then
        return 0
    fi
    if [[ -f "$window_id_file" ]]; then
        typeset recovered_id
        recovered_id=$(<"$window_id_file")
        if [[ "$recovered_id" == <-> ]]; then
            delegated_window_id="$recovered_id"
            return 0
        fi
    fi

    integer resolve_deadline=$(( SECONDS + 1 ))
    if (( resolve_deadline > cleanup_deadline )); then
        resolve_deadline=$cleanup_deadline
    fi
    run_bounded "$resolve_deadline" "$automation_stdout" "$automation_stderr" \
        "$osascript_bin" - "$ownership_token" "$tty_file" "$window_id_file" <<'RESOLVE_APPLESCRIPT'
use scripting additions

on persistValue(pathValue, textValue)
    do shell script "/usr/bin/printf '%s\\n' " & quoted form of textValue & " > " & quoted form of pathValue
end persistValue

on run argv
    set ownershipToken to item 1 of argv
    set ttyPath to item 2 of argv
    set windowIdPath to item 3 of argv
    set expectedTty to ""
    try
        set expectedTty to do shell script "/bin/cat " & quoted form of ttyPath
    end try

    set matchedIds to {}
    tell application "iTerm"
        repeat with candidateWindow in windows
            set candidateSession to current session of candidateWindow
            set tokenMatches to false
            try
                set tokenMatches to ((variable candidateSession named "user.icalctlFallback") is ownershipToken)
            end try
            set ttyMatches to false
            if expectedTty is not "" then
                try
                    set ttyMatches to ((tty of candidateSession) is expectedTty)
                end try
            end if
            if tokenMatches or ttyMatches then set end of matchedIds to id of candidateWindow
        end repeat
    end tell

    if (count of matchedIds) is not 1 then error "could not uniquely resolve delegated iTerm window"
    set resolvedId to item 1 of matchedIds
    my persistValue(windowIdPath, resolvedId as text)
    return resolvedId as text
end run
RESOLVE_APPLESCRIPT
    integer resolve_status=$?
    if (( resolve_status != 0 || ! -f "$window_id_file" )); then
        return 1
    fi
    typeset recovered_id
    recovered_id=$(<"$window_id_file")
    if [[ "$recovered_id" != <-> ]]; then
        return 1
    fi
    delegated_window_id="$recovered_id"
}

close_delegated_window() {
    local window_id=$1
    integer deadline=$2
    (( close_attempts += 1 ))
    rm -f -- "$close_focus_file" "$window_closed_file"

    integer close_call_deadline=$(( SECONDS + 1 ))
    if (( close_call_deadline > deadline )); then
        close_call_deadline=$deadline
    fi
    run_bounded "$close_call_deadline" "$close_stdout" "$close_stderr" \
        "$osascript_bin" - "$window_id" "$ownership_token" "$close_focus_file" "$window_closed_file" <<'CLOSE_APPLESCRIPT'
use framework "AppKit"
use scripting additions

on frontmostBundleIdentifier()
    set activeApplication to current application's NSWorkspace's sharedWorkspace()'s frontmostApplication()
    return (activeApplication's bundleIdentifier()) as text
end frontmostBundleIdentifier

on persistValue(pathValue, textValue)
    do shell script "/usr/bin/printf '%s\\n' " & quoted form of textValue & " > " & quoted form of pathValue
end persistValue

on run argv
    set delegatedWindowId to (item 1 of argv) as integer
    set ownershipToken to item 2 of argv
    set focusPath to item 3 of argv
    set closedPath to item 4 of argv
    my persistValue(focusPath, my frontmostBundleIdentifier())

    tell application "iTerm"
        if not (exists (first window whose id is delegatedWindowId)) then
            my persistValue(closedPath, "closed")
            return
        end if
        set delegatedWindow to first window whose id is delegatedWindowId
        set delegatedSession to current session of delegatedWindow
        if (variable delegatedSession named "user.icalctlFallback") is not ownershipToken then error "delegated iTerm window ownership changed"
        close delegatedWindow
    end tell
    my persistValue(closedPath, "closed")
end run
CLOSE_APPLESCRIPT
    integer close_status=$?

    if [[ ! -f "$window_closed_file" ]]; then
        integer reconcile_deadline=$(( SECONDS + 1 ))
        if (( reconcile_deadline > deadline )); then
            reconcile_deadline=$deadline
        fi
        run_bounded "$reconcile_deadline" "$automation_stdout" "$automation_stderr" \
            "$osascript_bin" - "$window_id" "$ownership_token" <<'RECONCILE_APPLESCRIPT'
on run argv
    set delegatedWindowId to (item 1 of argv) as integer
    set ownershipToken to item 2 of argv
    tell application "iTerm"
        if not (exists (first window whose id is delegatedWindowId)) then return "gone"
        set candidateWindow to first window whose id is delegatedWindowId
        set candidateSession to current session of candidateWindow
        if (variable candidateSession named "user.icalctlFallback") is not ownershipToken then return "different"
    end tell
    return "owned"
end run
RECONCILE_APPLESCRIPT
        integer reconcile_status=$?
        if (( reconcile_status == 0 )); then
            typeset reconciliation
            reconciliation=$(<"$automation_stdout")
            if [[ "$reconciliation" == "gone" || "$reconciliation" == "different" ]]; then
                : >"$window_closed_file"
                close_status=0
            fi
        fi
    else
        close_status=0
    fi

    integer restore_status=0
    if [[ -f "$close_focus_file" ]]; then
        integer restore_deadline=$(( SECONDS + 1 ))
        if (( restore_deadline > deadline )); then
            restore_deadline=$deadline
        fi
        restore_frontmost_application "$close_focus_file" "$restore_deadline"
        restore_status=$?
    fi

    if (( close_status != 0 )); then
        return "$close_status"
    fi
    return "$restore_status"
}

cleanup() {
    local original_status=$?
    trap - EXIT HUP INT TERM

    if (( active_pid > 0 )); then
        terminate_process "$active_pid"
        active_pid=0
    fi
    if [[ -n "$cancel_file" ]]; then
        : >"$cancel_file" 2>/dev/null || true
    fi
    interrupted=0
    recover_delegated_window_id >/dev/null 2>&1 || true
    if [[ -f "$window_closed_file" ]]; then
        delegated_window_id=""
    fi
    if [[ -n "$delegated_window_id" && -n "$close_stdout" ]] && \
        (( close_attempts < 2 && SECONDS < cleanup_deadline )); then
        close_delegated_window "$delegated_window_id" "$cleanup_deadline" >/dev/null 2>&1 || true
    fi
    if [[ -n "$temporary_directory" && -d "$temporary_directory" ]]; then
        rm -rf -- "$temporary_directory"
    fi

    exit "$original_status"
}

trap cleanup EXIT
trap handle_signal HUP INT TERM

if (( $# == 0 )); then
    print_usage
    exit "$USAGE_FAILURE"
fi

if [[ -n "${ICALCTL_ITERM_FALLBACK_OSASCRIPT:-}" ]]; then
    if [[ "${ICALCTL_ITERM_FALLBACK_TESTING:-}" != "1" ]]; then
        delegation_failure "the AppleScript override is available only to the test harness"
    fi
    osascript_bin="$ICALCTL_ITERM_FALLBACK_OSASCRIPT"
fi
if [[ ! -x "$osascript_bin" ]]; then
    delegation_failure "AppleScript is unavailable"
fi

typeset timeout_value=${ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS:-60}
if [[ "$timeout_value" != <-> ]] || (( timeout_value <= 0 )); then
    delegation_failure "the timeout must be a positive number of seconds"
fi
timeout_seconds=$timeout_value
operation_deadline=$(( SECONDS + timeout_seconds ))
cleanup_deadline=$(( operation_deadline + 4 ))

umask 077
typeset temporary_parent="${TMPDIR:-/tmp}"
temporary_parent="${temporary_parent%/}"
temporary_directory=$(mktemp -d "${temporary_parent}/icalctl-iterm.XXXXXXXX") || \
    delegation_failure "could not create a secure temporary directory"
chmod 700 "$temporary_directory" || delegation_failure "could not secure the temporary directory"

readonly stdout_file="${temporary_directory}/stdout"
readonly stderr_file="${temporary_directory}/stderr"
readonly status_file="${temporary_directory}/status"
readonly pending_status_file="${temporary_directory}/status.pending"
readonly command_started_file="${temporary_directory}/command-started"
window_id_file="${temporary_directory}/window-id"
tty_file="${temporary_directory}/tty"
gate_file="${temporary_directory}/gate"
cancel_file="${temporary_directory}/cancel"
create_focus_file="${temporary_directory}/create-focus"
close_focus_file="${temporary_directory}/close-focus"
window_closed_file="${temporary_directory}/window-closed"
automation_stdout="${temporary_directory}/automation.stdout"
automation_stderr="${temporary_directory}/automation.stderr"
close_stdout="${temporary_directory}/close.stdout"
close_stderr="${temporary_directory}/close.stderr"
ownership_token="icalctl-$(/usr/bin/uuidgen)"
: >"$stdout_file" || delegation_failure "could not create the stdout capture"
: >"$stderr_file" || delegation_failure "could not create the stderr capture"

run_bounded "$operation_deadline" "$automation_stdout" "$automation_stderr" \
    "$osascript_bin" -e 'id of application "iTerm"'
integer automation_status=$?
if (( automation_status == 124 )); then
    delegation_failure "checking iTerm availability timed out"
elif (( automation_status != 0 )); then
    delegation_failure "iTerm is not available or cannot be controlled"
fi
if (( interrupted )); then
    delegation_failure "delegation was interrupted"
fi

typeset -a quoted_command
typeset argument
for argument in "$@"; do
    quoted_command+=("${(qq)argument}")
done
readonly command_text="${(j: :)quoted_command}"
readonly task_body="if : >${(qq)command_started_file}; then ${command_text} >${(qq)stdout_file} 2>${(qq)stderr_file}; _icalctl_status=\$?; else _icalctl_status=${DELEGATION_FAILURE}; fi; printf '%s\\n' \"\$_icalctl_status\" >${(qq)pending_status_file}; mv -f -- ${(qq)pending_status_file} ${(qq)status_file}"
readonly bootstrap_body="umask 077; /usr/bin/tty >${(qq)tty_file}.pending && /bin/mv -f -- ${(qq)tty_file}.pending ${(qq)tty_file}; while [[ -d ${(qq)temporary_directory} && ! -e ${(qq)gate_file} && ! -e ${(qq)cancel_file} ]]; do /bin/sleep 0.05; done; if [[ ! -d ${(qq)temporary_directory} || -e ${(qq)cancel_file} ]]; then exit ${DELEGATION_FAILURE}; fi; ${task_body}"

run_bounded "$operation_deadline" "$automation_stdout" "$automation_stderr" \
    "$osascript_bin" - "$create_focus_file" "$window_id_file" "$ownership_token" "$bootstrap_body" "$tty_file" "$timeout_seconds" <<'CREATE_APPLESCRIPT'
use framework "AppKit"
use scripting additions

on frontmostBundleIdentifier()
    set activeApplication to current application's NSWorkspace's sharedWorkspace()'s frontmostApplication()
    return (activeApplication's bundleIdentifier()) as text
end frontmostBundleIdentifier

on persistValue(pathValue, textValue)
    do shell script "/usr/bin/printf '%s\\n' " & quoted form of textValue & " > " & quoted form of pathValue
end persistValue

on run argv
    set focusPath to item 1 of argv
    set windowIdPath to item 2 of argv
    set ownershipToken to item 3 of argv
    set bootstrapBody to item 4 of argv
    set timeoutSeconds to (item 6 of argv) as integer
    set launchCommand to "/bin/zsh -lc " & quoted form of bootstrapBody
    my persistValue(focusPath, my frontmostBundleIdentifier())
    set delegatedWindow to missing value

    try
        with timeout of timeoutSeconds seconds
            tell application "iTerm"
                set delegatedWindow to (create window with default profile command launchCommand)
                set delegatedSession to current session of delegatedWindow
                set variable delegatedSession named "user.icalctlFallback" to ownershipToken
                set visible of delegatedWindow to false
                if visible of delegatedWindow then error "delegated iTerm window remained visible"
                set delegatedWindowId to id of delegatedWindow
            end tell
            my persistValue(windowIdPath, delegatedWindowId as text)
        end timeout
    on error errorMessage number errorNumber
        if delegatedWindow is not missing value then
            tell application "iTerm"
                try
                    close delegatedWindow
                end try
            end tell
        end if
        error errorMessage number errorNumber
    end try

    return delegatedWindowId as text
end run
CREATE_APPLESCRIPT
automation_status=$?
recover_delegated_window_id >/dev/null 2>&1 || true

if [[ -f "$create_focus_file" ]]; then
    integer restore_deadline=$(( SECONDS + 1 ))
    if (( restore_deadline > operation_deadline )); then
        restore_deadline=$operation_deadline
    fi
    integer create_was_interrupted=$interrupted
    interrupted=0
    restore_frontmost_application "$create_focus_file" "$restore_deadline"
    integer focus_restore_status=$?
    interrupted=$create_was_interrupted
else
    integer focus_restore_status=1
fi

if (( automation_status == 124 )); then
    delegation_failure "creating the hidden iTerm window timed out"
elif (( automation_status != 0 )); then
    delegation_failure "the hidden iTerm window could not be created"
elif [[ "$delegated_window_id" != <-> ]]; then
    delegated_window_id=""
    delegation_failure "iTerm returned an invalid delegated window identifier"
elif (( focus_restore_status != 0 )); then
    delegation_failure "the frontmost application could not be restored after creating the hidden iTerm window"
fi
if (( interrupted )); then
    delegation_failure "delegation was interrupted"
fi

while [[ ! -f "$tty_file" ]]; do
    if (( interrupted )); then
        delegation_failure "delegation was interrupted before the command started"
    fi
    if (( SECONDS >= operation_deadline )); then
        delegation_failure "the local iTerm shell did not become ready"
    fi
    sleep 0.1
done

if ! : >"$gate_file"; then
    delegation_failure "the delegated command could not be released"
fi

while [[ ! -f "$status_file" ]]; do
    if (( interrupted )); then
        delegation_failure "delegation was interrupted; the command outcome may be uncertain"
    fi
    if (( SECONDS >= operation_deadline )); then
        delegation_failure "the delegated command timed out after ${timeout_seconds} seconds; its outcome may be uncertain"
    fi
    sleep 0.1
done

typeset delegated_status
delegated_status=$(<"$status_file")
if [[ "$delegated_status" != <-> ]] || (( delegated_status < 0 || delegated_status > 255 )); then
    delegation_failure "the delegated command returned a malformed status"
fi
if [[ ! -f "$command_started_file" ]]; then
    delegation_failure "the delegated command could not record its start; its outcome may be uncertain"
fi

cat "$stdout_file"
cat "$stderr_file" >&2
if (( interrupted )); then
    delegation_failure "delegation was interrupted after the command completed"
fi

close_delegated_window "$delegated_window_id" "$cleanup_deadline"
automation_status=$?
if (( automation_status == 124 )); then
    delegation_failure "closing the delegated iTerm window timed out"
elif (( automation_status != 0 )); then
    delegation_failure "the delegated iTerm window could not be closed or focus could not be restored"
fi
delegated_window_id=""

exit "$delegated_status"
