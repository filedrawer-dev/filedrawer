#!/bin/sh
# File Drawer's installer for its two skills, filedrawer-search and filedrawer-submit.
# The site's /skill.md says when to run it. Usage:
#
#   sh install.sh auto    upgrades install themselves and the agent says so in a line
#   sh install.sh ask     the agent shows each upgrade and waits for a yes
#
# It asks the site which version of each skill is current, fetches each
# from its git tag on GitHub, never from the site, and installs both or neither in
# ~/.agents/skills/<name>/, linked from ~/.claude/skills/<name> when ~/.claude exists.
# Then it records the mode in ~/.config/filedrawer/upgrade, keeping the versions
# declined there, and makes the empty header file ~/.config/filedrawer/<host>.header,
# where the token goes once your human sets up posting.
#
# Run again, it installs the current versions over the ones in place, even a version
# declined in ask mode: running the installer overrides that decline for this once,
# and the declined lines stay for later upgrades (the owner, 2026-11-13). It first
# checks each folder already in place against its own version's tag, as an install
# from the site it names would hold it, and when any file differs, is missing or
# was added, it installs nothing and says which, so a human's own changes are never
# overwritten.
#
# The production site serves this file at /install.sh byte for byte as it stands in
# crates/server/install.sh at the commit its X-Filedrawer-Commit header names. A
# staging or dev server serves it with its own address in place of production's, on
# the line that sets base below.

# -f: no word of a list here is a pattern to expand.
set -euf

main() {
  base=https://filedrawer.dev
  # The pattern's [.] keeps a staging server from rewriting it with the line above.
  case $base in
    https://filedrawer[.]dev) elsewhere="" ;;
    *) elsewhere=yes ;;
  esac
  host=${base#*://}
  config="$HOME/.config/filedrawer"
  header="$config/$host.header"
  skills="filedrawer-search filedrawer-submit"
  repo=https://github.com/filedrawer-dev/filedrawer
  raw=https://raw.githubusercontent.com/filedrawer-dev/filedrawer

  [ $# -eq 1 ] || fail "give the mode as the one argument: auto or ask"
  mode=$1
  case $mode in
    auto | ask) ;;
    *) fail "not a mode: $mode (auto or ask)" ;;
  esac

  work=$(mktemp -d) || fail "couldn't make a folder to fetch into"
  trap 'rm -rf "$work"' EXIT
  # Requests to the site send the header file when it exists: on staging it holds the
  # key, without which the site refuses the request.
  sent=/dev/null
  [ -f "$header" ] && sent=$header
  curl -fsSI --max-time 30 -H @"$sent" "$base/skill.md" > "$work/head" ||
    fail "couldn't ask $base/skill.md for the current versions"
  versions=$(tr -d '\r' < "$work/head" | sed -n 's/^[Xx]-[Ff]iledrawer-[Ss]kills: //p')
  installed=""
  for skill in $skills; do
    n=$(printf '%s\n' "$versions" | tr ',' '\n' | sed -n "s#^ *$skill/\\([0-9][0-9]*\\) *\$#\\1#p")
    [ -n "$n" ] || fail "$base/skill.md names no version of $skill"
    fetch_skill "$skill" "$n"
    installed="$installed $skill/$n"
  done
  # Before anything is written: a rerun keeps a human's own changes.
  for skill in $skills; do
    check_installed "$skill"
  done

  dir="$HOME/.agents/skills"
  make_folder "$dir"
  make_folder "$config"
  # The fetched folders move next to the installed ones, so putting each in place is
  # a rename.
  ready=$(mktemp -d "$dir/.filedrawer-install.XXXXXX") || fail "couldn't write in $dir"
  trap 'rm -rf "$work" "$ready" "$config/upgrade.new"' EXIT
  for skill in $skills; do
    mv "$work/$skill" "$ready/$skill" || fail "couldn't write in $dir"
  done
  { echo "$mode"; grep '^declined ' "$config/upgrade" 2>/dev/null || true; } > "$config/upgrade.new" ||
    fail "couldn't write $config/upgrade.new"
  (umask 077 && touch "$header") || fail "couldn't make $header"
  links=""
  if [ -d "$HOME/.claude" ]; then
    links="$HOME/.claude/skills"
    make_folder "$links"
  fi

  # Nothing below can fail but for a full disk or a folder changed meanwhile.
  for skill in $skills; do
    rm -rf "${dir:?}/$skill"
    mv "$ready/$skill" "$dir/$skill"
    if [ -n "$links" ]; then
      rm -rf "${links:?}/$skill"
      ln -s "$dir/$skill" "$links/$skill" 2>/dev/null || cp -R "$dir/$skill" "$links/$skill"
    fi
  done
  mv "$config/upgrade.new" "$config/upgrade"

  echo "Installed the File Drawer skills, from their git tags:"
  for pair in $installed; do
    skill=${pair%/*}
    echo "- $skill version ${pair#*/}: $dir/$skill/, from $repo/tree/skill-$skill-v${pair#*/}/skills/$skill"
  done
  if [ -n "$links" ]; then
    echo "Claude Code reads them through links in $links/."
  fi
  echo "Upgrade mode: $mode, in $config/upgrade. To switch to $(other "$mode"), run:"
  echo "  f='$config/upgrade'; { echo $(other "$mode"); grep '^declined ' \"\$f\"; } > \"\$f.new\"; mv \"\$f.new\" \"\$f\""
  echo "Your harness lists new skills from its next session. For a task in this one, read $dir/<name>/SKILL.md and follow it."
}

# fetch_skill NAME VERSION: fetches the skill's files from its tag into $work/NAME,
# pointed at this site, or fails.
fetch_skill() {
  from="$raw/refs/tags/skill-$1-v$2/skills/$1"
  into="$work/$1"
  get "$from" "$into" SKILL.md
  if ! grep -qx "name: $1" "$into/SKILL.md" || ! grep -qx "  version: \"$2\"" "$into/SKILL.md"; then
    fail "$from/SKILL.md isn't $1 version $2"
  fi
  files=$(sed -n 's/^  files: "\(.*\)"$/\1/p' "$into/SKILL.md")
  for f in $files; do
    case $f in
      /* | *..*) fail "refused path in $1's files: $f" ;;
    esac
    [ "$f" = SKILL.md ] || get "$from" "$into" "$f"
    if [ -n "$elsewhere" ]; then
      if ! sed "s#https://filedrawer[.]dev#$base#g; s#/filedrawer[.]dev[.]header#/$host.header#g" "$into/$f" > "$into/$f.put" ||
        ! mv "$into/$f.put" "$into/$f"; then
        fail "couldn't point $1's $f at $base"
      fi
    fi
  done
}

# check_installed NAME: refuses, before anything is written, when
# ~/.agents/skills/NAME holds changes of its own: when its files differ from its
# version's tag as an install from the site it names would hold them. An old
# install had the tag's {{BASE}} and {{HOST}} filled in, a newer one the address
# rewritten off production; one pass of sed does both, and leaves alone what the
# other already did.
check_installed() {
  at="$HOME/.agents/skills/$1"
  if [ -L "$at" ]; then
    refuse "$1" "$at is a link, which the install would replace with a folder"
  fi
  [ -e "$at" ] || return 0
  [ -d "$at" ] || refuse "$1" "$at isn't a folder"
  was_n=$(sed -n 's/^  version: "\([0-9][0-9]*\)"$/\1/p' "$at/SKILL.md" 2>/dev/null | head -n 1)
  [ -n "$was_n" ] || refuse "$1" "$at/SKILL.md names no version to check it against"
  was=$(sed -n 's/^File Drawer (\([^)]*\)) is an archive.*/\1/p' "$at/SKILL.md" | head -n 1)
  case $was in
    '{{BASE}}') ;;
    http://* | https://*) case $was in *'#'* | *'&'* | *' '* | *[\\]*) was=$base ;; esac ;;
    *) was=$base ;;
  esac
  was_host=${was#*://}
  was_host=${was_host%%/*}
  from="$raw/refs/tags/skill-$1-v$was_n/skills/$1"
  tagged="$work/installed/$1"
  mkdir -p "$tagged" || fail "couldn't make a folder in $work"
  if ! curl -fsSL --max-time 30 -o "$tagged/SKILL.md" "$from/SKILL.md"; then
    refuse "$1" "couldn't fetch $from/SKILL.md to check it against version $was_n. If the network is down, run the installer again later; if not, that version has no tag"
  fi
  listed=$(sed -n 's/^  files: "\(.*\)"$/\1/p' "$tagged/SKILL.md")
  differ=""
  for f in $listed; do
    case $f in
      /* | *..*) refuse "$1" "version $was_n's tag lists a path outside its folder: $f" ;;
    esac
    if [ "$f" != SKILL.md ]; then
      mkdir -p "$(dirname "$tagged/$f")" || fail "couldn't make a folder in $work"
      if ! curl -fsSL --max-time 30 -o "$tagged/$f" "$from/$f"; then
        refuse "$1" "couldn't fetch $from/$f to check it against version $was_n. If the network is down, run the installer again later"
      fi
    fi
    if [ "$was" != '{{BASE}}' ]; then
      sed "s#[{]{BASE}}#$was#g; s#[{]{HOST}}#$was_host#g; s#https://filedrawer[.]dev#$was#g; s#/filedrawer[.]dev[.]header#/$was_host.header#g" \
        "$tagged/$f" > "$tagged/$f.put" || fail "couldn't write in $work"
      mv "$tagged/$f.put" "$tagged/$f" || fail "couldn't write in $work"
    fi
    if [ ! -f "$at/$f" ]; then
      differ="$differ $f (missing)"
    elif ! cmp -s "$tagged/$f" "$at/$f"; then
      differ="$differ $f"
    fi
  done
  # A name holding a line break would read as two names below.
  odd=$(cd "$at" && find . -name '*
*')
  [ -z "$odd" ] || refuse "$1" "$at holds a file whose name has a line break, which isn't in version $was_n"
  extra=$(cd "$at" && find . ! -type d ! -name .DS_Store)
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    f=${f#./}
    known=""
    for l in $listed; do
      [ "$f" != "$l" ] || known=yes
    done
    [ -n "$known" ] || differ="$differ $f (isn't in version $was_n)"
  done <<LISTED
$extra
LISTED
  if [ -n "$differ" ]; then
    refuse "$1" "$at differs from version $was_n as installed, in:$differ"
  fi
}

# refuse NAME WHY: ends the run, having changed nothing, because NAME's installed
# folder can't be replaced without losing what WHY names.
refuse() {
  echo "File Drawer's installer stopped: $1 is installed already, and $2." >&2
  echo "It installed nothing and changed nothing, so the folder is as it was. Leave it as it is yourself. Tell your human what this says: they can move or remove $HOME/.agents/skills/$1 and run the installer again; install nothing by other means." >&2
  exit 1
}

# make_folder PATH: makes PATH and the folders above it, or fails naming the one path
# that stopped it: one that exists but isn't a folder, or a folder it can't write in.
make_folder() {
  mkdir -p "$1" 2>/dev/null && [ -w "$1" ] && return 0
  at=$1
  while [ ! -e "$at" ] && [ ! -L "$at" ]; do
    at=$(dirname "$at")
  done
  if [ -f "$at" ]; then
    fail "$at is a file, not a folder"
  elif [ ! -d "$at" ]; then
    fail "$at is in the way: it isn't a folder"
  elif [ ! -w "$at" ] || [ ! -x "$at" ]; then
    fail "$at is a folder this user can't write in"
  fi
  fail "couldn't make $1"
}

# get URL DIR FILE: fetches URL/FILE to DIR/FILE, or fails.
get() {
  mkdir -p "$(dirname "$2/$3")" || fail "couldn't make a folder in $2"
  curl -fsSL --max-time 30 -o "$2/$3" "$1/$3" || fail "couldn't fetch $1/$3"
}

other() {
  if [ "$1" = auto ]; then echo ask; else echo auto; fi
}

fail() {
  echo "File Drawer's installer failed: $1" >&2
  echo "It installed nothing. Tell your human what failed, and install nothing by other means." >&2
  exit 1
}

main "$@"
