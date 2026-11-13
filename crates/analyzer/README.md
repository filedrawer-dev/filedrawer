# filedrawer-analyzer

The library that reads a [File Drawer](https://filedrawer.dev) entry by [docs/entry-format.md](https://github.com/filedrawer-dev/filedrawer/blob/main/docs/entry-format.md).
It lists what would refuse the entry and what the site couldn't read, reads its Results table, and renders its card.
The filedrawer.dev server and the `entry-check` command in the [`filedrawer`](https://crates.io/crates/filedrawer) package both run it, so an entry that passes offline passes on the site.

It does no I/O.
The code is AGPL-3.0-only.
