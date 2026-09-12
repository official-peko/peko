---
name: peko
description: Check an iOS or Android app against App Store and Google Play policy before submitting it. Use when working on a mobile app and the task involves store review, rejection, compliance, privacy manifests, permissions, purchases, account deletion, or a rejection letter from Apple or Google.
---

# Peko

Peko checks a mobile app against published store policy and says what would
get it refused, with the file and line and the guideline behind it.

Two tiers. The lint runs on this machine, calls no model, and needs no
account. The audit sends the parts of the source each rule reads to a model
and judges the guidelines that prose alone decides, and it needs a paid plan.

## Before anything else

Check it is installed and which one is on the path:

```
command -v peko && peko --version
```

If it is missing, install it:

```
curl -fsSL https://peko.so/install.sh | sh
```

If `peko` resolves to something that is not this tool, use the full path the
installer printed. Another program with the same name is a real thing that
happens.

## Start here, always

```
peko init
peko lint --all
```

`peko init` writes `.pekorc.json` and works out the platform. `peko lint`
prints findings with a file, a line, the rule id, and what to change. Exit
code 1 means something at or above the fail level was found.

Run the lint before offering an opinion about store compliance. It is free,
it is fast, and it is evidence.

## Reading a finding

Each one carries a severity, a rule id, a location, and a fix. The rule id is
stable and worth quoting to the user. `error` means the store refuses it.
`warning` means a reviewer is likely to raise it. `info` is worth knowing.

Do not fix a finding by silencing it. `peko override` exists for a rule that
genuinely does not apply, it requires a written reason, and it refuses on
rules the store enforces regardless.

## When facts are missing

Some rules cannot decide from code alone: whether the app is for children,
where it ships, whether it sells data.

```
peko facts
```

This one needs a key, unlike the lint. Without one it stops and says so, which
is easy to read as the project being broken rather than the command needing an
account.

Answer them in the `facts` block of `.pekorc.json`. **Ask the user rather than
guessing.** A wrong answer here does not produce a wrong finding, it produces
silence, and silence reads like a pass. `distributes_in` takes only `US`,
`US-CA`, and `eu`.

## The audit tier

```
peko audit          # says what it would read, uses nothing
peko audit --yes    # runs it
```

It takes minutes. The first run against an app opens a cycle and uses one of
a fixed number per month, so **ask the user before the first one.** Every run
against the same app for the next seven days is included, so once a cycle is
open, re-running after a fix costs nothing and is worth doing.

`peko audit` without `--yes` says whether a run would be free. Read it before
asking, rather than asking every time.

It refuses while the free checks are failing, and while a fact a rule needs
has no answer. Fix those first.

## A rejection letter

If the user has been refused by Apple or Google, this is the first thing to
run. It is free and calls no model.

```
peko diagnose < rejection.txt
```

It names the guidelines cited, the rules behind them, and what to change. It
also names any guideline no rule covers yet. **Do not skip that part when
summarising**: it is the one the user will otherwise miss.

## Telling Peko what happened

After the store decides, record it:

```
peko outcome approved
peko outcome rejected --sections 3.1.1 --notes-file rejection.txt
```

This is how the rule database learns whether a finding was right. Offer it
once the user knows the outcome.

## In CI

```yaml
- uses: official-peko/peko@v1
  with:
    path: .
    fail-on: error
```

No key needed for the lint. Findings land on the pull request through Code
Scanning.

## What not to do

- Do not claim an app is compliant. Peko reports what it checked, coverage is
  partial, and the reports say so. Report what it found.
- Do not treat a finding as legal advice. Each cites the policy section it
  comes from; quote that and let the user decide.
- Do not open a new cycle without asking. The first audit against an app
  spends one of a limited number. A re-run inside an open cycle does not, and
  refusing to re-run after a fix is its own mistake.
- Do not invent rule ids. Use the ones the output prints.
- Do not edit the `facts` block on a guess.

## Everything else

`peko --help` lists every command. The full reference is at
<https://peko.so/docs>.
