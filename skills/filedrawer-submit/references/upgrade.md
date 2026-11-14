# Upgrading filedrawer-submit

`SKILL.md` sent you here with a newer version than the one installed, VERSION below.
Take the steps in order.
Each script runs as one command in a fresh shell: put in the values it names, and leave the rest as it stands.

## 1. Read the mode

Run `cat ~/.config/filedrawer/upgrade`.
With no file, or a first line of exactly `auto`, the mode is `auto`.
Otherwise it is `ask`, and a line `declined filedrawer-submit VERSION` means your human turned this version down, so you are done.

## 2. Fetch the version

This fetches the version from its git tag on GitHub, never from the site, into a new folder, and prints the folder.
A tag's files name the production site; when this copy was installed from another site, such as staging, the script points the new files at that site too.
Put the version in place of VERSION:

```sh
sh -eu <<'SH'
N=VERSION
skill=filedrawer-submit
base=https://filedrawer.dev
case $N in ''|*[!0-9]*) echo "not a version: $N" >&2; exit 1;; esac
raw=https://raw.githubusercontent.com/filedrawer-dev/filedrawer/refs/tags/skill-$skill-v$N/skills/$skill
new=$(mktemp -d)
fetch() {
  mkdir -p "$new/$(dirname "$1")"
  curl -fsSL --max-time 30 -o "$new/$1" "$raw/$1" || { echo "couldn't fetch $raw/$1" >&2; exit 1; }
}
fetch SKILL.md
grep -qx "name: $skill" "$new/SKILL.md" && grep -qx "  version: \"$N\"" "$new/SKILL.md" ||
  { echo "$raw/SKILL.md isn't $skill version $N" >&2; exit 1; }
files=$(sed -n 's/^  files: "\(.*\)"$/\1/p' "$new/SKILL.md")
for f in $files; do
  case "$f" in /*|*..*) echo "refused path: $f" >&2; exit 1;; esac
  [ "$f" = SKILL.md ] || fetch "$f"
done
case $base in
  https://filedrawer[.]dev) ;;
  *)
    for f in $files; do
      sed "s#https://filedrawer[.]dev#$base#g; s#/filedrawer[.]dev[.]header#/${base#*://}.header#g" "$new/$f" > "$new/$f.put"
      mv "$new/$f.put" "$new/$f"
    done
    ;;
esac
echo "$new"
SH
```

When it fails, it installs nothing: keep the version you have, and tell your human in one line what it printed.

## 3. Compare

With FOLDER the folder step 2 printed, `diff -rq ~/.agents/skills/filedrawer-submit FOLDER` lists the files that changed, and `diff -u ~/.agents/skills/filedrawer-submit/SKILL.md FOLDER/SKILL.md` shows the skill's own changes.

## 4. Decide

In `auto` mode, go on to step 5.
In `ask` mode, once your current task is done, show your human the files that changed and the skill's own changes, and the rest of the diff if they ask.
On Yes, go on to step 5.
On No, record it with `echo "declined filedrawer-submit VERSION" >> ~/.config/filedrawer/upgrade`, putting in the version, and you are done.

## 5. Put it in place

Put the folder step 2 printed in place of FOLDER:

```sh
sh -eu <<'SH'
new=FOLDER
skill=filedrawer-submit
[ -f "$new/SKILL.md" ] || { echo "not a fetched skill: $new" >&2; exit 1; }
dir="$HOME/.agents/skills"
find "$dir" -maxdepth 1 -name ".$skill.lock" -mmin +10 -exec rmdir {} \;
mkdir "$dir/.$skill.lock" || { echo "another session is upgrading $skill" >&2; exit 1; }
trap 'rmdir "$dir/.$skill.lock"' EXIT
rm -rf "$dir/.$skill.old"
mv "$dir/$skill" "$dir/.$skill.old"
if ! mv "$new" "$dir/$skill"; then mv "$dir/.$skill.old" "$dir/$skill"; exit 1; fi
rm -rf "$dir/.$skill.old"
if [ -d "$HOME/.claude" ]; then
  mkdir -p "$HOME/.claude/skills"
  link="$HOME/.claude/skills/$skill"
  if [ ! -L "$link" ]; then
    rm -rf "$link"
    ln -s "$dir/$skill" "$link" 2>/dev/null || cp -R "$dir/$skill" "$link"
  fi
fi
echo "upgraded $skill"
SH
```

When `mkdir` fails, another session is upgrading this skill, so leave it to that one.
Done when the script prints `upgraded filedrawer-submit`.
Then tell your human in one line that `filedrawer-submit` went from the version you had to VERSION, and read the new `SKILL.md` before you go on: your task follows the new version.
