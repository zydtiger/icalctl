#!/bin/zsh

set -u
set -o pipefail

readonly repository_root="${0:A:h:h}"
readonly helper="${repository_root}/skills/icalctl/scripts/icalctl-iterm-fallback.sh"
readonly skill="${repository_root}/skills/icalctl/SKILL.md"
readonly mock_osascript="${repository_root}/tests/fixtures/iterm-fallback/mock-osascript.sh"
readonly echo_args="${repository_root}/tests/fixtures/iterm-fallback/echo-args.sh"
readonly suite_directory=$(mktemp -d "${TMPDIR:-/tmp}/icalctl-iterm-tests.XXXXXXXX")

integer assertions=0
integer failures=0
integer case_number=0
typeset case_directory
typeset case_tmp
typeset case_log
typeset case_stdout
typeset case_stderr
integer run_status=0

cleanup() {
    rm -rf -- "$suite_directory"
}
trap cleanup EXIT HUP INT TERM

fail() {
    print -u2 -- "not ok - $1"
    (( failures += 1 ))
}

pass() {
    print -- "ok - $1"
}

assert_equal() {
    local description=$1
    local expected=$2
    local actual=$3
    (( assertions += 1 ))
    if [[ "$actual" == "$expected" ]]; then
        pass "$description"
    else
        fail "$description (expected ${(qq)expected}, got ${(qq)actual})"
    fi
}

assert_contains() {
    local description=$1
    local needle=$2
    local file=$3
    (( assertions += 1 ))
    if grep -Fq -- "$needle" "$file"; then
        pass "$description"
    else
        fail "$description (missing ${(qq)needle})"
    fi
}

assert_not_contains() {
    local description=$1
    local needle=$2
    local file=$3
    (( assertions += 1 ))
    if grep -Fq -- "$needle" "$file"; then
        fail "$description (unexpected ${(qq)needle})"
    else
        pass "$description"
    fi
}

assert_files_equal() {
    local description=$1
    local expected=$2
    local actual=$3
    (( assertions += 1 ))
    if cmp -s "$expected" "$actual"; then
        pass "$description"
    else
        fail "$description"
        diff -u "$expected" "$actual" >&2 || true
    fi
}

new_case() {
    (( case_number += 1 ))
    case_directory="${suite_directory}/case-${case_number}"
    case_tmp="${case_directory}/tmp"
    case_log="${case_directory}/osascript.log"
    case_stdout="${case_directory}/stdout"
    case_stderr="${case_directory}/stderr"
    mkdir -p "$case_tmp"
    : >"$case_log"
}

run_helper() {
    local mode=$1
    shift

    TMPDIR="$case_tmp" \
        ICALCTL_ITERM_FALLBACK_TESTING=1 \
        ICALCTL_ITERM_FALLBACK_OSASCRIPT="$mock_osascript" \
        MOCK_OSASCRIPT_MODE="$mode" \
        MOCK_OSASCRIPT_LOG="$case_log" \
        "$helper" "$@" >"$case_stdout" 2>"$case_stderr"
    run_status=$?
}

assert_temporary_files_cleaned() {
    local description=$1
    (( assertions += 1 ))
    if [[ -z "$(find "$case_tmp" -mindepth 1 -print -quit)" ]]; then
        pass "$description"
    else
        fail "$description"
        find "$case_tmp" -mindepth 1 -print >&2
    fi
}

typeset compiled_script_directory="${suite_directory}/compiled-applescript"
mkdir -p "$compiled_script_directory"
for script_label in RESTORE_APPLESCRIPT RESOLVE_APPLESCRIPT CLOSE_APPLESCRIPT RECONCILE_APPLESCRIPT CREATE_APPLESCRIPT; do
    typeset source_file="${compiled_script_directory}/${script_label}.applescript"
    typeset compiled_file="${compiled_script_directory}/${script_label}.scpt"
    sed -n "/<<'${script_label}'/,/^${script_label}$/p" "$helper" | sed '1d;$d' >"$source_file"
    (( assertions += 1 ))
    if /usr/bin/osacompile -o "$compiled_file" "$source_file" >/dev/null 2>&1; then
        pass "${script_label} compiles as AppleScript"
    else
        fail "${script_label} compiles as AppleScript"
        /usr/bin/osacompile -o "$compiled_file" "$source_file" >&2 || true
    fi
done

new_case
"$helper" >"$case_stdout" 2>"$case_stderr"
run_status=$?
assert_equal "missing executable uses usage-failure status" 64 "$run_status"
assert_equal \
    "usage identifies the helper executable" \
    "usage: icalctl-iterm-fallback.sh ICALCTL_EXECUTABLE [ARG ...]" \
    "$(<"$case_stderr")"
assert_equal "usage does not write stdout" "" "$(<"$case_stdout")"

assert_contains \
    "delegated live-write failures are documented as uncertain" \
    'if a delegated live write returns `125` after the command may have been' \
    "$skill"
assert_contains \
    "uncertain delegated live writes cannot be replayed" \
    "Do not rerun it directly," \
    "$skill"
assert_contains \
    "uncertain delegated live writes require fresh confirmation" \
    "then obtain fresh explicit confirmation for" \
    "$skill"

new_case
run_helper success /bin/zsh -c 'printf "delegated-out\n\n"; printf "delegated-err\n\n\n" >&2'
typeset expected_stdout="${case_directory}/expected-stdout"
typeset expected_stderr="${case_directory}/expected-stderr"
printf "delegated-out\n\n" >"$expected_stdout"
printf "delegated-err\n\n\n" >"$expected_stderr"
assert_equal "delegated success returns zero" 0 "$run_status"
assert_files_equal "delegated stdout is preserved byte-for-byte" "$expected_stdout" "$case_stdout"
assert_files_equal "delegated stderr is preserved byte-for-byte" "$expected_stderr" "$case_stderr"
assert_contains "temporary directory is owner-only" "temporary-mode:700" "$case_log"
assert_contains "stdout capture is owner-only" "stdout-mode:600" "$case_log"
assert_contains "stderr capture is owner-only" "stderr-mode:600" "$case_log"
assert_contains "only the delegated window id is closed" "close:4242" "$case_log"
assert_contains "close targets a window by id" "first window whose id is delegatedWindowId" "$case_log"
assert_contains "the default profile is given an explicit local command" "explicit-command-override" "$case_log"
assert_contains "the delegated window receives an ownership token" "ownership-set:4242" "$case_log"
assert_not_contains "the window is hidden before ownership and id bookkeeping" "invalid-create-order" "$case_log"
assert_not_contains "the helper never types into a profile-defined shell" "write text commandText" "$helper"
assert_temporary_files_cleaned "success removes temporary files"

new_case
run_helper success /bin/zsh -c 'print -u2 -r -- expected-error; exit 7'
typeset expected_nonzero_stderr="${case_directory}/expected-stderr"
printf "expected-error\n" >"$expected_nonzero_stderr"
assert_equal "delegated nonzero status is preserved" 7 "$run_status"
assert_files_equal "delegated nonzero stderr is preserved exactly" "$expected_nonzero_stderr" "$case_stderr"
assert_temporary_files_cleaned "nonzero command removes temporary files"

new_case
typeset injection_marker="${case_directory}/injected"
typeset -a dangerous_arguments=(
    "two words"
    "single'quote"
    '\$(touch should-not-run)'
    "; touch ${injection_marker}"
    $'line one\nline two'
)
run_helper success "$echo_args" "${dangerous_arguments[@]}"
typeset expected_arguments="${case_directory}/expected-arguments"
: >"$expected_arguments"
for argument in "${dangerous_arguments[@]}"; do
    print -r -- "ARG:${argument}" >>"$expected_arguments"
done
assert_equal "quoted argument command succeeds" 0 "$run_status"
assert_files_equal "all arguments survive shell delegation exactly" "$expected_arguments" "$case_stdout"
(( assertions += 1 ))
if [[ ! -e "$injection_marker" ]]; then
    pass "shell metacharacters are not evaluated"
else
    fail "shell metacharacters are not evaluated"
fi

new_case
run_helper missing_iterm /usr/bin/true
assert_equal "missing iTerm uses delegation-failure status" 125 "$run_status"
assert_contains "missing iTerm reports availability failure" "iTerm is not available or cannot be controlled" "$case_stderr"
assert_not_contains "missing iTerm never creates a window" "create" "$case_log"
assert_temporary_files_cleaned "missing iTerm leaves no temporary files"

new_case
run_helper create_failure /usr/bin/true
assert_equal "AppleScript creation failure uses delegation-failure status" 125 "$run_status"
assert_contains "AppleScript creation failure is explicit" "hidden iTerm window could not be created" "$case_stderr"
assert_temporary_files_cleaned "AppleScript failure removes temporary files"

new_case
run_helper visibility_failure /usr/bin/true
assert_equal "visibility failure uses delegation-failure status" 125 "$run_status"
assert_contains "mock reaches the runtime visibility failure" "visibility-failed:4242" "$case_log"
assert_contains "visibility failure runs the AppleScript error close" "error-close:4242" "$case_log"
assert_not_contains "visibility failure never releases the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "visibility failure removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper pre_hide_hang /usr/bin/true
assert_equal "pre-hide automation timeout uses delegation-failure status" 125 "$run_status"
assert_contains "pre-hide timeout is allowed to reach its AppleScript error handler" "internal-timeout-before-hide" "$case_log"
assert_contains "pre-hide timeout runs the AppleScript error close" "error-close:4242" "$case_log"
assert_contains "pre-hide timeout restores the captured frontmost application" "focus-restored:com.example.FrontApp" "$case_log"
assert_not_contains "pre-hide timeout never persists an unsafe window id" "window-id-persisted:4242" "$case_log"
assert_not_contains "pre-hide timeout never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "pre-hide timeout removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper pre_hide_error_close_failure /usr/bin/true
assert_equal "failed pre-hide error close uses delegation-failure status" 125 "$run_status"
assert_contains "failed error close is exercised after the internal timeout" "error-close-failed:4242" "$case_log"
assert_contains "unowned pre-hide cleanup reports its conservative skip" "window ownership could not be verified" "$case_stderr"
assert_not_contains "unowned pre-hide cleanup never guesses a window to close" "close:4242" "$case_log"
assert_contains "failed pre-hide close still restores the captured frontmost application" "focus-restored:com.example.FrontApp" "$case_log"
assert_not_contains "failed pre-hide error close never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "failed pre-hide error close removes temporary files"

new_case
TMPDIR="$case_tmp" \
    ICALCTL_ITERM_FALLBACK_TESTING=1 \
    ICALCTL_ITERM_FALLBACK_OSASCRIPT="$mock_osascript" \
    ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=20 \
    MOCK_OSASCRIPT_MODE=pre_hide_signal_wait \
    MOCK_OSASCRIPT_LOG="$case_log" \
    "$helper" /usr/bin/true >"$case_stdout" 2>"$case_stderr" &
integer pre_hide_interrupted_pid=$!
integer pre_hide_signal_waits=0
while ! grep -Fq "waiting-before-hide" "$case_log"; do
    sleep 0.1
    (( pre_hide_signal_waits += 1 ))
    if (( pre_hide_signal_waits > 30 )); then
        break
    fi
done
kill -TERM "$pre_hide_interrupted_pid"
wait "$pre_hide_interrupted_pid"
run_status=$?
assert_equal "pre-hide interruption uses delegation-failure status" 125 "$run_status"
assert_not_contains "pre-hide interruption occurs before ownership is set" "ownership-set:4242" "$case_log"
assert_contains "pre-hide interruption reports unverified ownership" "window ownership could not be verified" "$case_stderr"
assert_not_contains "pre-hide interruption never closes an unverified window" "close:4242" "$case_log"
assert_contains "pre-hide interruption restores the captured frontmost application" "focus-restored:com.example.FrontApp" "$case_log"
assert_not_contains "pre-hide interruption never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "pre-hide interruption removes temporary files"

new_case
run_helper post_hide_persist_hang /usr/bin/true
assert_equal "post-hide persistence timeout uses delegation-failure status" 125 "$run_status"
assert_contains "post-hide timeout occurs only after the window is hidden" "hidden:4242" "$case_log"
assert_contains "post-hide timeout runs the AppleScript error close" "error-close:4242" "$case_log"
assert_not_contains "post-hide timeout never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "post-hide timeout removes temporary files"

new_case
TMPDIR="$case_tmp" \
    ICALCTL_ITERM_FALLBACK_TESTING=1 \
    ICALCTL_ITERM_FALLBACK_OSASCRIPT="$mock_osascript" \
    ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=20 \
    MOCK_OSASCRIPT_MODE=create_delay \
    MOCK_OSASCRIPT_LOG="$case_log" \
    "$helper" /usr/bin/true >"$case_stdout" 2>"$case_stderr" &
integer interrupted_pid=$!
integer create_waits=0
while ! grep -Fq "window-id-persisted:4242" "$case_log"; do
    sleep 0.1
    (( create_waits += 1 ))
    if (( create_waits > 30 )); then
        break
    fi
done
kill -TERM "$interrupted_pid"
wait "$interrupted_pid"
run_status=$?
assert_equal "interruption uses delegation-failure status" 125 "$run_status"
assert_contains "interruption closes only its delegated window" "close:4242" "$case_log"
assert_temporary_files_cleaned "interruption removes temporary files"

new_case
run_helper restore_signals_parent /usr/bin/true
assert_equal "a signal racing with successful focus restoration is preserved" 125 "$run_status"
assert_contains "the restore mock signals the helper before returning" "restore-signalled-parent:com.example.FrontApp" "$case_log"
assert_not_contains "a post-restore signal prevents command execution" "task-started" "$case_log"
assert_contains "post-restore interruption closes the owned delegated window" "window-closed:4242" "$case_log"
assert_temporary_files_cleaned "post-restore interruption removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper availability_hang /usr/bin/true
assert_equal "hung availability check uses delegation-failure status" 125 "$run_status"
assert_contains "hung availability check is bounded" "checking iTerm availability timed out" "$case_stderr"
assert_not_contains "hung availability check never creates a window" "create" "$case_log"
assert_temporary_files_cleaned "hung availability check removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper create_hang /usr/bin/true
assert_equal "hung window creation uses delegation-failure status" 125 "$run_status"
assert_contains "hung window creation is bounded" "creating the hidden iTerm window timed out" "$case_stderr"
assert_contains "hung creation recovers and closes the persisted window id" "close:4242" "$case_log"
assert_not_contains "hung creation never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "hung creation removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper create_effect_before_response_hang /usr/bin/true
assert_equal "create effect followed by an AppleEvent hang fails safely" 125 "$run_status"
assert_contains "effect-before-response creation is exercised" "create-effect-before-response" "$case_log"
assert_contains "effect-before-response reports missing ownership evidence" "window ownership could not be verified" "$case_stderr"
assert_not_contains "effect-before-response never guesses a window to close" "close:4242" "$case_log"
assert_not_contains "the guarded command is cancelled before execution" "task-started" "$case_log"
assert_temporary_files_cleaned "effect-before-response failure removes temporary files"

new_case
run_helper create_error_close_failure /usr/bin/true
assert_equal "a failed create-time error close uses delegation-failure status" 125 "$run_status"
assert_contains "the create error close failure is exercised" "error-close-failed:4242" "$case_log"
assert_contains "cleanup resolves the orphan candidate by ownership" "resolve-by-ownership" "$case_log"
assert_contains "cleanup closes the owned window" "close:4242" "$case_log"
assert_contains "owned cleanup records the close effect" "window-closed:4242" "$case_log"
assert_temporary_files_cleaned "failed create-time close removes temporary files"

new_case
run_helper success /usr/bin/true
integer hidden_line=$(grep -nF "hidden:4242" "$case_log" | head -1 | cut -d: -f1)
integer finished_line=$(grep -nF "task-started" "$case_log" | head -1 | cut -d: -f1)
(( assertions += 1 ))
if (( hidden_line > 0 && finished_line > hidden_line )); then
    pass "the local command remains gated until the window is hidden"
else
    fail "the local command remains gated until the window is hidden"
fi
assert_temporary_files_cleaned "gated execution removes temporary files"

new_case
run_helper user_focus_change /usr/bin/true
assert_equal "a user focus change does not fail delegation" 0 "$run_status"
assert_contains "a third application remains frontmost" "third-app-left-frontmost" "$case_log"
assert_contains "focus restoration does not steal focus from a third app" "focus-not-stolen" "$case_log"
assert_temporary_files_cleaned "third-app focus case removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper timeout /usr/bin/true
assert_equal "timeout uses delegation-failure status" 125 "$run_status"
assert_contains "timeout is reported" "timed out after 1 seconds" "$case_stderr"
assert_contains "timeout closes only its delegated window" "close:4242" "$case_log"
assert_temporary_files_cleaned "timeout removes temporary files"

new_case
run_helper malformed_status /usr/bin/true
assert_equal "malformed status uses delegation-failure status" 125 "$run_status"
assert_contains "malformed status is reported" "malformed status" "$case_stderr"
assert_contains "malformed status closes only its delegated window" "close:4242" "$case_log"
assert_temporary_files_cleaned "malformed status removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper missing_status /usr/bin/true
assert_equal "missing status eventually fails" 125 "$run_status"
assert_contains "missing status follows the finite timeout" "timed out after 1 seconds" "$case_stderr"
assert_temporary_files_cleaned "missing status removes temporary files"

new_case
run_helper close_failure /usr/bin/true
assert_equal "window close failure uses delegation-failure status" 125 "$run_status"
assert_contains "window close failure is reported" "delegated iTerm window could not be closed" "$case_stderr"
assert_contains "close failure still targets only the delegated window" "close:4242" "$case_log"
assert_temporary_files_cleaned "close failure removes temporary files"

new_case
integer close_hang_started=$SECONDS
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper close_hang /usr/bin/true
integer close_hang_elapsed=$(( SECONDS - close_hang_started ))
assert_equal "hung window close uses delegation-failure status" 125 "$run_status"
assert_contains "hung window close is bounded" "closing the delegated iTerm window timed out" "$case_stderr"
assert_contains "hung close targets only the delegated window" "close:4242" "$case_log"
integer close_hang_attempts=$(grep -c '^close:4242$' "$case_log")
assert_equal "hung close uses at most one bounded retry" 2 "$close_hang_attempts"
(( assertions += 1 ))
if (( close_hang_elapsed <= 5 )); then
    pass "hung close stays within the global timeout plus cleanup allowance"
else
    fail "hung close stays within the global timeout plus cleanup allowance (elapsed ${close_hang_elapsed}s)"
fi
assert_temporary_files_cleaned "hung close removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=1 run_helper close_after_effect_hang /usr/bin/true
assert_equal "close hang after effect preserves delegated command success" 0 "$run_status"
assert_contains "close-after-effect state is recorded" "window-closed:4242" "$case_log"
assert_contains "close-after-effect ambiguity is reconciled" "reconcile:4242" "$case_log"
assert_contains "focus is restored after close takes effect" "focus-restored:com.example.FrontApp" "$case_log"
integer effected_close_attempts=$(grep -c '^close:4242$' "$case_log")
assert_equal "known completed close is not retried" 1 "$effected_close_attempts"
assert_temporary_files_cleaned "close-after-effect hang removes temporary files"

new_case
run_helper window_id_reused /usr/bin/true
assert_equal "a reused numeric window id is treated as the original window being gone" 0 "$run_status"
assert_contains "reused id is reconciled without another close" "reconcile:4242" "$case_log"
integer reused_close_attempts=$(grep -c '^close:4242$' "$case_log")
assert_equal "ownership mismatch prevents a close retry against another window" 1 "$reused_close_attempts"
assert_temporary_files_cleaned "reused-id case removes temporary files"

new_case
run_helper close_focus_failure /usr/bin/true
assert_equal "post-close focus restoration failure uses delegation-failure status" 125 "$run_status"
assert_contains "window close effect is recorded before focus failure" "window-closed:4242" "$case_log"
assert_contains "post-close focus restoration is attempted" "focus-restore-failed:com.example.FrontApp" "$case_log"
integer focus_failed_close_attempts=$(grep -c '^close:4242$' "$case_log")
assert_equal "closed window is not retried after focus failure" 1 "$focus_failed_close_attempts"
assert_temporary_files_cleaned "post-close focus failure removes temporary files"

new_case
run_helper focus_failure /usr/bin/true
assert_equal "focus restoration failure uses delegation-failure status" 125 "$run_status"
assert_contains "focus restoration failure is exercised at runtime" "focus-restore-failed" "$case_log"
assert_contains "focus restoration failure closes the created window" "close:4242" "$case_log"
assert_not_contains "focus restoration failure cancels before command start" "task-started" "$case_log"
assert_temporary_files_cleaned "focus restoration failure removes temporary files"

new_case
ICALCTL_ITERM_FALLBACK_TIMEOUT_SECONDS=2 run_helper focus_restore_hang /usr/bin/true
assert_equal "hung create-time focus restoration uses delegation-failure status" 125 "$run_status"
assert_contains "hung focus restoration is bounded" "focus-restore-hung:com.example.FrontApp" "$case_log"
assert_contains "cleanup retains independent time to close the known window" "close:4242" "$case_log"
assert_not_contains "hung focus restoration never starts the guarded command" "task-started" "$case_log"
assert_temporary_files_cleaned "hung focus restoration removes temporary files"

new_case
run_helper success /usr/bin/true
assert_contains "frontmost application is captured without System Events" "NSWorkspace's sharedWorkspace()'s frontmostApplication()" "$case_log"
assert_contains "focus is restored only when the frontmost bundle changed" "if activeBundleIdentifier is targetBundleIdentifier then return" "$helper"
assert_contains "focus is not stolen after a user switches elsewhere" "if activeBundleIdentifier is not iTermBundleIdentifier then return" "$helper"
assert_contains "focus restoration verifies activation success" "if restored is false then error" "$helper"
assert_contains "success restores the pre-create frontmost application" "focus-restored:com.example.FrontApp" "$case_log"
assert_not_contains "iTerm is never explicitly activated" 'tell application "iTerm" to activate' "$case_log"
integer hidden_runtime_line=$(grep -nF "hidden:4242" "$case_log" | head -1 | cut -d: -f1)
integer command_runtime_line=$(grep -nF "task-started" "$case_log" | head -1 | cut -d: -f1)
(( assertions += 1 ))
if (( hidden_runtime_line < command_runtime_line )); then
    pass "runtime command completes only after the delegated window is hidden"
else
    fail "runtime command completes only after the delegated window is hidden"
fi
assert_temporary_files_cleaned "focus-preservation run removes temporary files"

if (( failures > 0 )); then
    print -u2 -- "${failures} of ${assertions} assertions failed"
    exit 1
fi

print -- "${assertions} assertions passed"
