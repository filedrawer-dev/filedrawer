#!/bin/sh
# One fake of ssh, gh, getent, curl or ssh-keygen, for conftest's FakeHost.
#
# Each fake is a link to this file at ROOT/bin/TOOL. It's /bin/sh because a Python
# fake cost 40 ms of CPU a call, about a fifth of bin/tests' CPU (#164).
#
# Notes the call in ROOT/log, keeps an ssh call's stdin in ROOT/sent/<n>, and
# answers from the latest rule in ROOT/rules/TOOL.sh that matches, which FakeHost
# writes as `case` statements that exit; an unmatched ssh `test -f` or `test -x`
# answers 1, as a fresh server would.
#
# $stdin holds what the call read, for a rule's `stdin`; dash drops any NUL in it,
# which only that matching sees.

case $0 in
*/*) ;;
*)
	echo "fake_tool.sh: called as $0, not by a path to ROOT/bin/TOOL" >&2
	exit 127
	;;
esac
# A relative PATH entry gives a relative $0, so resolve the folder.
bin=$(cd "${0%/*}" && pwd) || exit 127
root=${bin%/*}
tool=${0##*/}
stdin=
# Another tool reads stdin only when an argument names it, as curl's `-K -`, gh's
# `--input -` or a `@-` value do, or for gh secret set. Reading it then matters: a
# writer piped into a fake that exits unread can die of SIGPIPE, which pipefail
# turns into the pipeline's failure.
if [ "$tool" != ssh ]; then
	reads=
	for arg; do
		case $arg in
		- | *@-) reads=1 ;;
		esac
	done
	if [ "$tool" = gh ] && [ "$1 $2" = "secret set" ]; then reads=1; fi
	if [ -n "$reads" ]; then
		stdin=$(
			cat
			printf x
		)
		stdin=${stdin%x}
	fi
fi
if [ "$tool" = ssh ]; then
	# Skip the options, and the value of each one that takes one (ssh(1)), then the
	# host: the words after it are the remote command line.
	while [ $# -gt 0 ]; do
		case $1 in
		-[BbcDEeFIiJLlmOoPpQRSWw])
			shift
			[ $# -gt 0 ] && shift
			;;
		-*) shift ;;
		*) break ;;
		esac
	done
	[ $# -gt 0 ] && shift
fi
IFS=' '
line="$*"
printf '%s %s\n' "$tool" "$line" >>"$root/log"

if [ "$tool" = ssh ]; then
	set -- "$root"/sent/*
	if [ -e "$1" ]; then n=$(($# + 1)); else n=1; fi
	# noclobber creates the file only if it's new, so two calls at once take a number
	# each.
	while ! (set -C && : >"$root/sent/$n") 2>/dev/null; do n=$((n + 1)); done
	sent="$root/sent/$n"
	cat >"$sent"
	stdin=$(
		cat "$sent"
		printf x
	)
	stdin=${stdin%x}
fi

# A rule that answers exits here; one that runs the line locally reads $sent.
# shellcheck source=/dev/null
. "$root/rules/$tool.sh"

if [ "$tool" = gh ] && [ "$1" = secret ]; then
	case $2 in
	list) LC_ALL=C ls -A "$root/secrets" ;;
	set)
		if [ $# -eq 5 ] && [ "$4" = --env ] && [ "$5" = staging ]; then
			printf %s "$stdin" >"$root/secrets/$3"
		fi
		;;
	esac
fi
if [ "$tool" = ssh ]; then
	case $line in
	"test -f "* | "test -x "*) exit 1 ;;
	esac
fi
exit 0
