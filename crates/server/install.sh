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
# The production site serves this file at /install.sh byte for byte as it stands in
# crates/server/install.sh at the commit its X-Filedrawer-Commit header names. A
# staging or dev server serves it with its own address in place of production's, on
# the line that sets base below.

set -eu

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

  mkdir -p "$HOME/.agents/skills" "$config" 2>/dev/null ||
    fail "couldn't make $HOME/.agents/skills and $config"
  dir="$HOME/.agents/skills"
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
    { mkdir -p "$links" 2>/dev/null && [ -w "$links" ]; } || fail "couldn't write in $links"
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
