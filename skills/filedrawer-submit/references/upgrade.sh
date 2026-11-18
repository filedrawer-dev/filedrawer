#!/bin/sh
# Upgrades File Drawer's skill filedrawer-submit, where its installer put it. The
# skill's SKILL.md says when to run it, with the version the site named. Usage:
#
#   sh upgrade.sh VERSION        in auto mode, installs VERSION; in ask mode, shows
#                                what changed and installs nothing
#   sh upgrade.sh VERSION yes    in ask mode, installs VERSION: the human said yes
#   sh upgrade.sh VERSION no     in ask mode, records that the human declined VERSION
#
# The mode is the first line of ~/.config/filedrawer/upgrade, which the installer
# wrote: auto, also when there is no file, and otherwise ask. The script fetches
# VERSION from its git tag on GitHub, never from the site, and puts the folder in
# place of ~/.agents/skills/filedrawer-submit/, linked from ~/.claude/skills/ when
# ~/.claude exists. A copy of the skill kept anywhere else is left alone.
#
# Its last lines of output tell the agent what it did and what to do next.
#
# A staging or dev server serves this file with its own address in place of
# production's, on the line that sets base below, and so does a copy installed from
# one.

# -f: no word of a list here is a pattern to expand.
set -euf

main() {
  skill=filedrawer-submit
  base=https://filedrawer.dev
  # The pattern's [.] keeps a staging server from rewriting it with the line above.
  case $base in
    https://filedrawer[.]dev) elsewhere="" ;;
    *) elsewhere=yes ;;
  esac
  host=${base#*://}
  dir="$HOME/.agents/skills"
  at="$dir/$skill"
  config="$HOME/.config/filedrawer"
  raw=https://raw.githubusercontent.com/filedrawer-dev/filedrawer
  tag=https://github.com/filedrawer-dev/filedrawer/tree

  # A signal ends the run through the EXIT traps below, which free the lock.
  trap 'exit 1' HUP INT TERM
  [ $# -ge 1 ] && [ $# -le 2 ] || fail "give the version, and yes or no after it when your human answered"
  n=$1
  answer=${2:-}
  case $n in
    '' | *[!0-9]* | ??????????*) fail "not a version: $n" ;;
  esac
  case $answer in
    '' | yes | no) ;;
    *) fail "not an answer: $answer (yes or no)" ;;
  esac

  # Only the installer's copy is upgraded: the swap below assumes its folders.
  self=$(cd "$(dirname "$0")/.." && pwd -P) || fail "couldn't find the folder this script is in"
  placed=""
  if [ -d "$at" ]; then
    for known in "$at" "$HOME/.claude/skills/$skill"; do
      if [ -d "$known" ] && [ "$(cd "$known" && pwd -P)" = "$self" ]; then
        placed=yes
      fi
    done
  fi
  if [ -z "$placed" ]; then
    echo "$skill version $n is out, at $tag/skill-$skill-v$n/skills/$skill."
    echo "This copy, in $self, isn't one File Drawer's installer placed, so nothing was changed. Tell your human that in one line, and go on with your task."
    exit 0
  fi

  have=$(sed -n 's/^  version: "\([0-9][0-9]*\)"$/\1/p' "$at/SKILL.md" | head -n 1)
  [ -n "$have" ] || fail "$at/SKILL.md names no version"
  if [ "$n" -le "$have" ]; then
    echo "$skill is at version $have already, so there is nothing to upgrade. Go on with your task."
    exit 0
  fi

  mode=auto
  if [ -f "$config/upgrade" ] && [ "$(head -n 1 "$config/upgrade")" != auto ]; then
    mode=ask
  fi
  if [ "$mode" = auto ] && [ "$answer" = no ]; then
    fail "the upgrade mode is auto, which installs without asking, so there is no answer to record. To be asked first, your human switches the mode to ask in $config/upgrade"
  fi
  if [ "$answer" = no ]; then
    # A mode file edited by hand may end without a line end.
    if [ -n "$(tail -c 1 "$config/upgrade")" ]; then
      echo >> "$config/upgrade" || fail "couldn't write $config/upgrade"
    fi
    echo "declined $skill $n" >> "$config/upgrade" || fail "couldn't write $config/upgrade"
    echo "Recorded that your human declined $skill version $n, in $config/upgrade. Nothing was installed. Go on with your task."
    exit 0
  fi
  if [ "$mode" = ask ] && [ -z "$answer" ] && grep -qx "declined $skill $n" "$config/upgrade"; then
    echo "Your human declined $skill version $n before, so nothing was changed. Don't ask again. Go on with your task."
    exit 0
  fi

  # The version is fetched beside the installed folder, so putting it in place is a
  # rename, which can't stop halfway as a copy from another disk can.
  work=$(mktemp -d "$dir/.$skill.new.XXXXXX") || fail "couldn't make a folder in $dir to fetch into"
  trap 'rm -rf "$work"' EXIT
  fetch_skill

  if [ "$mode" = ask ] && [ -z "$answer" ]; then
    echo "$skill version $n is out. The upgrade mode is ask, so nothing was installed. What changed from version $have:"
    echo
    diff -rq "$at" "$work/$skill" || true
    echo
    diff -u "$at/SKILL.md" "$work/$skill/SKILL.md" || true
    echo
    echo "Finish your current task first. Then show your human the files that changed and the changes to SKILL.md, both above, and ask your human whether to install version $n."
    echo "On Yes, run: sh '$at/references/upgrade.sh' $n yes"
    echo "On No, run: sh '$at/references/upgrade.sh' $n no"
    echo "The question stays open until your human answers Yes or No: keeping version $have without asking is not one of the outcomes."
    echo "Ask it in a reply of its own, once any other question you asked is answered, or in your next reply when that one goes unanswered. If your human doesn't answer this one, ask it once more, then leave it without running no: the next task's check brings it up again."
    exit 0
  fi

  find "$dir" -maxdepth 1 -name ".$skill.lock" -mmin +10 -exec rmdir {} \;
  mkdir "$dir/.$skill.lock" 2>/dev/null ||
    fail "another session is upgrading $skill, so leave it to that one"
  trap 'rm -rf "$work"; rmdir "$dir/.$skill.lock"' EXIT
  rm -rf "$dir/.$skill.old"
  mv "$at" "$dir/.$skill.old" || fail "couldn't move $at aside"
  if ! mv "$work/$skill" "$at"; then
    if ! mv "$dir/.$skill.old" "$at"; then
      echo "File Drawer's upgrade of $skill failed, and it couldn't put the installed copy back: that copy is now in $dir/.$skill.old. Tell your human, who can move it back to $at." >&2
      exit 1
    fi
    fail "couldn't write in $dir"
  fi
  rm -rf "$dir/.$skill.old"
  # The skill is upgraded from here on, so nothing below may end the run before it
  # says so.
  link="$HOME/.claude/skills/$skill"
  linked=yes
  if [ -d "$HOME/.claude" ] && [ ! -L "$link" ]; then
    {
      mkdir -p "$HOME/.claude/skills" && rm -rf "$link" &&
        { ln -s "$at" "$link" || cp -R "$at" "$link"; }
    } 2>/dev/null || linked=""
  fi

  echo "upgraded $skill from version $have to $n, from $tag/skill-$skill-v$n/skills/$skill."
  if [ -z "$linked" ]; then
    echo "Claude Code's copy, $link, couldn't be replaced, so Claude Code may still read the old version there. Tell your human that too."
  fi
  echo "Tell your human that in one line, without asking. Then read $at/SKILL.md again before you go on: your task follows the new version."
}

# fetch_skill: fetches version $n of the skill from its tag into $work/$skill,
# pointed at this site, or fails.
fetch_skill() {
  from="$raw/refs/tags/skill-$skill-v$n/skills/$skill"
  into="$work/$skill"
  get SKILL.md
  if ! grep -qx "name: $skill" "$into/SKILL.md" || ! grep -qx "  version: \"$n\"" "$into/SKILL.md"; then
    fail "$from/SKILL.md isn't $skill version $n"
  fi
  files=$(sed -n 's/^  files: "\(.*\)"$/\1/p' "$into/SKILL.md")
  for f in $files; do
    case $f in
      /* | *..*) fail "refused path: $f" ;;
    esac
    [ "$f" = SKILL.md ] || get "$f"
    if [ -n "$elsewhere" ]; then
      if ! sed "s#https://filedrawer[.]dev#$base#g; s#/filedrawer[.]dev[.]header#/$host.header#g" "$into/$f" > "$into/$f.put" ||
        ! mv "$into/$f.put" "$into/$f"; then
        fail "couldn't point $f at $base"
      fi
    fi
  done
}

# get FILE: fetches FILE of the tag to $into/FILE, or fails.
get() {
  mkdir -p "$(dirname "$into/$1")" || fail "couldn't make a folder in $work"
  curl -fsSL --max-time 30 -o "$into/$1" "$from/$1" || fail "couldn't fetch $from/$1"
}

fail() {
  echo "File Drawer's upgrade of $skill failed: $1." >&2
  echo "It changed nothing: keep the version you have, tell your human in one line what failed, and go on with your task." >&2
  exit 1
}

main "$@"
