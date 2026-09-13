# Peko

Peko checks an iOS or Android project against the App Store and Google Play
policies, and reports what a reviewer is likely to refuse.

This repository holds the part you run. It reads files on your machine, calls
no model, and needs no account.

## Install

macOS and Linux:

```bash
curl -fsSL https://peko.so/install.sh | sh
```

Windows, in PowerShell:

```powershell
irm https://peko.so/install.ps1 | iex
```

Both work out the build for your machine, download it from the release, check
it against the checksum published beside it, and install one file. Neither
needs an account. Read
[install.sh](https://peko.so/install.sh) or
[install.ps1](https://peko.so/install.ps1) before you run either, which is the
sensible thing to do with any script somebody tells you to pipe into a shell.

### By hand

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

Or start at <https://peko.so/start>, which gives you a code and walks you
through it. That path uses `peko start`, which sets the project up, checks
it, and tells the page how it went so it can follow along. What it sends is
the step, the platform, the version, and how many findings there were at each
severity. No code, no file names, and no project name. Every step of that page
also works by hand, so nothing is lost by ignoring it.

The first run needs no account and no server. The rule database ships inside
the binary, so the check itself reads only files on your machine. What the
run reports afterwards, and how to turn that off, is below.

### Something to try it on

`examples/ios-app` and `examples/android-app` carry real problems on purpose:
an empty purpose string, no privacy manifest, `QUERY_ALL_PACKAGES`, cleartext
traffic, a target API below what Google Play accepts.

```bash
git clone https://github.com/official-peko/peko
peko lint --all peko/examples/ios-app
```

Three findings on that one and five on the Android one. Fix them and watch
them go.

`examples/ios-audit` is the interesting one. It passes the lint with nothing
to report and it is not compliant: its paywall never offers to restore a
purchase, its account cannot be deleted, and its metrics call sends the
account email to a third party. No file states any of that, which is what the
audit tier is for.

```bash
peko start --demo    # fetches both and runs them, guided
```

## Keeping it current

```bash
peko update
```

It tells you when a newer release exists, once a day at most, on stderr and
never in JSON or SARIF output. `PEKO_NO_UPDATE_CHECK=1` stops it asking, and
it never asks in CI.

The rule database updates on its own and needs no command: the binary ships
with one, fetches a newer one when it can, and falls back to what it has when
it cannot.

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
          version: v1.11.0
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

**Lint** runs here, free, on every push. It finds what a file can prove: a
removed API, a missing declaration, a permission with no stated reason.

**Audit** runs on the server before a release. It reads code and judges the
guidelines a file cannot settle on its own.

`peko lint` uses the server when it has a key, because the rule database
there is current without upgrading this binary. Without a key it runs here.

## What it sends

The check works without a network. The rule database ships inside the binary,
and a run with no key reads only files on your machine.

Unless you say otherwise, a finished run reports the command, how long it
took, what it exited with, the version, the operating system, and the shape of
the project: how many files, which platform, which framework, and how many
findings by severity.

It never sends source, file paths, the project name, the bundle id, or your
key. The send times out after 1.5 seconds and fails silently, so it cannot
slow a run down or break one.

Three ways to switch it off, and any one of them is enough:

    PEKO_TELEMETRY=off      in the environment
    "telemetry": false      in .pekorc.json
    DO_NOT_TRACK=1          the convention other tools honour

With a key, `peko lint` calls the server instead, because the rule database
there is current without upgrading this binary. That send does include the
files being checked.

## What a finding means, and does not

A finding names the policy section it comes from. It is a reading of a
published rule, not a decision by a reviewer. Peko is quiet on 342 of 346
published apps, and the four it is not quiet on hold real gaps that a person
confirmed.

That says the rules do not fire wrongly on working code. It says nothing
about a fault the corpus does not hold.

## Licence

Apache 2.0. See `LICENSE`.
