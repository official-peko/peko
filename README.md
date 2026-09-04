# Peko

Peko checks an iOS or Android project against the App Store and Google Play
policies, and reports what a reviewer is likely to refuse.

This repository holds the part you run. It reads files on your machine, calls
no model, and needs no account.

```bash
peko init
peko lint --all
```

The first run works offline. The rule database ships inside the binary.

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
