# Peko

Peko checks an iOS or Android project against the App Store and Google Play
policies, and reports what a reviewer is likely to refuse.

This repository holds the part you run. It reads files on your machine, calls
no model, and needs no account.

## Install

Pick the file for your machine.

| Machine | File |
|---|---|
| macOS, Apple silicon | `peko-aarch64-apple-darwin` |
| macOS, Intel | `peko-x86_64-apple-darwin` |
| Linux, x86_64 | `peko-x86_64-unknown-linux-gnu` |
| Linux, arm64 | `peko-aarch64-unknown-linux-gnu` |
| Windows | `peko-x86_64-pc-windows-msvc.exe` |

Download it under its own name, check it against the checksum beside it, then
rename it and put it on your path.

```bash
name=peko-aarch64-apple-darwin
base=https://github.com/official-peko/peko/releases/latest/download

curl -fsSLO "$base/$name"
curl -fsSLO "$base/$name.sha256"
shasum -a 256 -c "$name.sha256"

chmod +x "$name"
sudo mv "$name" /usr/local/bin/peko
```

The checksum file names the file it belongs to, so keep the downloaded name
until the check passes. On Linux use `sha256sum -c` in place of `shasum -a
256 -c`.

If you download through a browser rather than with `curl`, macOS marks the
file and refuses to run it. Clear the mark first.

```bash
xattr -d com.apple.quarantine peko
```

## Run it

```bash
cd your-project
peko init
peko lint --all
```

The first run works offline. The rule database ships inside the binary, and
nothing leaves your machine.

## In a pull request

```yaml
permissions:
  contents: read
  security-events: write

jobs:
  compliance:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: official-peko/peko@v1
        with:
          version: v1.3.0
```

Every finding lands on the line it belongs to, in the diff, through GitHub
Code Scanning. The mechanical checks run on the runner, so this needs no
account and no key.

Pin the version. A new rule that fails a build which passed yesterday, with
nobody changing anything, is how a check gets switched off.

## What is here

| Crate | What it does |
|---|---|
| `peko-cli` | `peko`, the command a developer runs |
| `peko-check` | The mechanical check engine |
| `peko-parse` | Readers for plist, `AndroidManifest.xml`, gradle, pbxproj, lockfiles, and compiled bundles |
| `peko-rules` | The rule schema, the database loader, and the rules compiled in |
| `peko-report` | The report, as JSON and as Markdown |

`rules/` holds 47 mechanical rules. Each one cites the section of the policy
it comes from, so you can read the source yourself.

## What is not here

The interpretive tier reads code with a language model and judges the
subjective guidelines, for example minimum functionality and spam. It runs on
a server because it costs money per run, and `peko audit` reaches it.

Also not here: the pipeline that keeps the rules current against the
published policies, the curated dependency knowledge that answers the privacy
forms, and the corpus that measures whether any of it is right.

## The two tiers

**Lint** runs here, free, offline, on every push. It finds what a file can
prove: a removed API, a missing declaration, a permission with no stated
reason.

**Audit** runs on the server before a release. It reads code and judges the
guidelines a file cannot settle on its own.

`peko lint` uses the server when it has a key, because the rule database
there is current without upgrading this binary. Without a key it runs here.

## What a finding means, and does not

A finding names the policy section it comes from. It is a reading of a
published rule, not a decision by a reviewer. Peko is quiet on 342 of 346
published apps, and the four it is not quiet on hold real gaps that a person
confirmed.

That says the rules do not fire wrongly on working code. It says nothing
about a fault the corpus does not hold.

## Licence

Apache 2.0. See `LICENSE`.
