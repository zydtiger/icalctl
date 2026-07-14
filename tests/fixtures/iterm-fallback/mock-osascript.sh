#!/bin/zsh

set -u

typeset mode=${MOCK_OSASCRIPT_MODE:-success}
typeset log=${MOCK_OSASCRIPT_LOG:?MOCK_OSASCRIPT_LOG is required}
typeset state_file="${log:h}/window-state"

if [[ "${1:-}" == "-e" ]]; then
    print -r -- "availability" >>"$log"
    case "$mode" in
        missing_iterm) exit 1 ;;
        availability_hang) sleep 20; exit 0 ;;
        *) print -r -- "com.googlecode.iterm2"; exit 0 ;;
    esac
fi

typeset script
script=$(<&0)

if grep -Fq "runningApplicationsWithBundleIdentifier" <<<"$script"; then
    typeset bundle_identifier=$2
    print -r -- "restore:${bundle_identifier}" >>"$log"
    integer restore_count=$(grep -c '^restore:' "$log")
    case "$mode" in
        focus_failure)
            print -r -- "focus-restore-failed:${bundle_identifier}" >>"$log"
            exit 1
            ;;
        focus_restore_hang)
            if (( restore_count == 1 )); then
                print -r -- "focus-restore-hung:${bundle_identifier}" >>"$log"
                sleep 20
            fi
            ;;
        close_focus_failure)
            if (( restore_count > 1 )); then
                print -r -- "focus-restore-failed:${bundle_identifier}" >>"$log"
                exit 1
            fi
            ;;
        user_focus_change)
            print -r -- "third-app-left-frontmost" >>"$log"
            print -r -- "focus-not-stolen" >>"$log"
            exit 0
            ;;
    esac
    print -r -- "focus-restored:${bundle_identifier}" >>"$log"
    exit 0
fi

if grep -Fq "could not uniquely resolve delegated iTerm window" <<<"$script"; then
    typeset window_id_file=$4
    print -r -- "resolve-by-ownership" >>"$log"
    if [[ -f "$state_file" && "$(<"$state_file")" == "owned" ]]; then
        print -r -- "4242" >"$window_id_file"
        print -r -- "4242"
        exit 0
    fi
    exit 1
fi

if grep -Fq 'return "owned"' <<<"$script"; then
    print -r -- "reconcile:${2}" >>"$log"
    if [[ ! -f "$state_file" || "$(<"$state_file")" == "gone" ]]; then
        print -r -- "gone"
    elif [[ "$(<"$state_file")" == "different" ]]; then
        print -r -- "different"
    else
        print -r -- "owned"
    fi
    exit 0
fi

run_bootstrap() {
    local bootstrap_body=$1
    local command_started_file=$2
    (
        /bin/zsh -lc "$bootstrap_body"
        if [[ -f "$command_started_file" ]]; then
            print -r -- "task-started" >>"$log"
        else
            print -r -- "bootstrap-cancelled" >>"$log"
        fi
    ) &
}

if grep -Fq "create window with default profile command launchCommand" <<<"$script"; then
    typeset focus_file=$2
    typeset window_id_file=$3
    typeset ownership_token=$4
    typeset bootstrap_body=$5
    typeset tty_file=$6
    typeset capture_directory=${window_id_file:h}
    print -r -- "com.example.FrontApp" >"$focus_file"
    {
        print -r -- "create"
        print -r -- "focus-persisted:com.example.FrontApp"
        print -r -- "explicit-command-override"
        print -r -- "ownership-token:${ownership_token}"
        print -r -- "temporary-mode:$(stat -f '%Lp' "$capture_directory")"
        print -r -- "stdout-mode:$(stat -f '%Lp' "${capture_directory}/stdout")"
        print -r -- "stderr-mode:$(stat -f '%Lp' "${capture_directory}/stderr")"
        print -r -- "create-script-begin"
        print -r -- "$script"
        print -r -- "create-script-end"
    } >>"$log"

    case "$mode" in
        create_failure)
            print -r -- "create-failed" >>"$log"
            exit 1
            ;;
    esac

    print -r -- "created:4242" >>"$log"
    print -r -- "owned" >"$state_file"
    print -r -- "/dev/ttys999" >"$tty_file"
    case "$mode" in
        timeout|missing_status)
            : >"${capture_directory}/command-started"
            print -r -- "command-started:4242" >>"$log"
            ;;
        malformed_status)
            : >"${capture_directory}/command-started"
            print -r -- "not-a-status" >"${capture_directory}/status"
            print -r -- "command-started:4242" >>"$log"
            ;;
        *)
            run_bootstrap "$bootstrap_body" "${capture_directory}/command-started"
            ;;
    esac

    case "$mode" in
        create_effect_before_response_hang)
            print -r -- "create-effect-before-response" >>"$log"
            sleep 20
            exit 0
            ;;
        create_error_close_failure)
            print -r -- "error-close-failed:4242" >>"$log"
            exit 1
            ;;
        pre_hide_hang)
            print -r -- "internal-timeout-before-hide" >>"$log"
            print -r -- "gone" >"$state_file"
            print -r -- "error-close:4242" >>"$log"
            exit 1
            ;;
        visibility_failure)
            print -r -- "visibility-failed:4242" >>"$log"
            print -r -- "gone" >"$state_file"
            print -r -- "error-close:4242" >>"$log"
            exit 1
            ;;
        post_hide_persist_hang)
            print -r -- "hidden:4242" >>"$log"
            print -r -- "gone" >"$state_file"
            print -r -- "error-close:4242" >>"$log"
            exit 1
            ;;
    esac

    print -r -- "ownership-set:4242" >>"$log"
    print -r -- "hidden:4242" >>"$log"
    print -r -- "4242" >"$window_id_file"
    print -r -- "window-id-persisted:4242" >>"$log"
    case "$mode" in
        create_hang) sleep 20; exit 0 ;;
        create_delay) sleep 1; print -r -- "4242"; exit 0 ;;
        *) print -r -- "4242"; exit 0 ;;
    esac
fi

typeset window_id=${2:-missing}
typeset ownership_token=${3:-}
typeset focus_file=${4:-}
typeset closed_file=${5:-}
if [[ -n "$focus_file" ]]; then
    print -r -- "com.example.FrontApp" >"$focus_file"
fi
{
    print -r -- "close:${window_id}"
    print -r -- "close-ownership:${ownership_token}"
    print -r -- "close-focus-persisted:com.example.FrontApp"
    print -r -- "close-script-begin"
    print -r -- "$script"
    print -r -- "close-script-end"
} >>"$log"

case "$mode" in
    close_failure)
        exit 1
        ;;
    close_after_effect_failure|close_focus_failure)
        print -r -- "window-closed:${window_id}" >>"$log"
        print -r -- "gone" >"$state_file"
        exit 1
        ;;
    close_hang)
        sleep 20
        exit 0
        ;;
    close_after_effect_hang)
        print -r -- "window-closed:${window_id}" >>"$log"
        print -r -- "gone" >"$state_file"
        sleep 20
        exit 0
        ;;
    window_id_reused)
        print -r -- "different" >"$state_file"
        exit 1
        ;;
    *)
        print -r -- "window-closed:${window_id}" >>"$log"
        print -r -- "gone" >"$state_file"
        print -r -- "closed" >"$closed_file"
        exit 0
        ;;
esac
