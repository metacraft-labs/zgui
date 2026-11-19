# metacraft-labs/zgui

This repository is Metacraft Labs' fork of [zortax/zgui](https://github.com/zortax/zgui), adapted
for [IsoNim](https://github.com/metacraft-labs/isonim). IsoNim drives zgui's retained document from
Nim through the C ABI in [isonim-zgui](https://github.com/metacraft-labs/isonim-zgui).

Read `CLAUDE.md` and `CONTRIBUTING.md` too: zgui's own conventions apply to every change here.

## Branches

- `isonim` is the product branch, the GitHub default, and the only branch that carries Metacraft
  changes. Consumers pin an exact revision of it.
- `main` mirrors upstream's `main`. Nothing else is committed to it.

## Tracking upstream: merges, not rebases

The adaptation branch merges upstream's `main` into `isonim` at regular intervals. It does not
rebase onto it. This is a declared exception to the fork-branch rule in the Metacraft branching
policy: the changes here are developed for IsoNim and are not prepared as upstream pull requests,
so a linear patch stack has no reader, while a rebase would rewrite revisions that consumers pin.

To take upstream's latest work:

```sh
git fetch upstream
git switch isonim
git merge upstream/main
```

Resolve conflicts in favour of upstream's design where the two overlap, run the tests of every
crate the merge touched, and push `isonim`.

## Changes made here

Every change carries a test in the crate it changes. No pull requests or issues are opened on the
upstream repository from this fork.
