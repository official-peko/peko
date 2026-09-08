# Example projects

Four small projects, in two pairs. Each pair is the same app on iOS and on
Android, and each pair exists to exercise one tier.

| Project | Platform | Lint | What it is for |
|---|---|---|---|
| `ios-app` | iOS | fails | The lint tier |
| `android-app` | Android | fails | The lint tier |
| `ios-audit` | iOS | passes | The audit tier |
| `android-audit` | Android | passes | The audit tier |

## The lint pair

`ios-app` and `android-app` carry planted mechanical problems: an empty
purpose string, a missing privacy manifest, `QUERY_ALL_PACKAGES`, a target
API level below the Play minimum. A file states each one, so a check that
reads files finds it.

Run them to see what the free tier reports:

    peko lint --all examples/ios-app

The comments in those two say what is wrong and why. That is on purpose. They
are there to be read.

## The audit pair

`ios-audit` and `android-audit` are the same note taking app, Harbor, on both
platforms. Both pass the lint with nothing to report. Neither is compliant.

Every problem in them is one that no file states:

- The paywall sells a subscription and never offers to restore it.
- Settings can create an account and sign out of it, and cannot delete it.
- Sign in is Google or Facebook, with no equivalent that hides the address.
- The metrics call sends the account email to a third party host.
- The Android build logs the session token.

No comment in either project points at any of these. A comment saying "this
paywall has no restore" would let a model find it by reading the comment, and
a demo that works that way proves nothing. The code reads like ordinary app
code, because that is the case the audit tier is for.

Run one:

    peko audit examples/ios-audit          # says what it would read
    peko audit examples/ios-audit --yes    # runs it

Both hold a full `facts` block in `.pekorc.json`. The audit refuses to start
while a fact a rule needs has no answer, so a project without one stops before
it reads anything.

Both ship to `["US", "US-CA", "eu"]`, which is every value the rules read for
`distributes_in`. Those three switch on 33 more rules between them, which is
most of the privacy and consent side of the database. Any other string parses
and matches nothing.

## Why both pairs exist

A lint that reports a problem in every project is worth as little as one that
never reports any. The audit pair is what proves the first tier can say
nothing, and the lint pair is what proves it can say something. The action
workflow runs all four on Linux, macOS, and Windows, and checks each against
the outcome it is supposed to have.
