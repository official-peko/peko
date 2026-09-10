# Agent skills

`peko/SKILL.md` teaches a coding agent how to use Peko: which command to reach
for, when to ask before spending, and what not to claim.

## Install it

Copy the folder into wherever your agent reads skills from. For Claude Code
that is `.claude/skills/` in a project, or `~/.claude/skills/` for every
project:

    mkdir -p ~/.claude/skills
    curl -fsSL https://peko.so/skill.md -o ~/.claude/skills/peko/SKILL.md

Or from a checkout:

    cp -r skills/peko ~/.claude/skills/peko

Other agents read skills from their own directory. The file is plain markdown
with a name and a description at the top, so it ports.

## Why a skill and not a README

An agent that has read this runs the free lint before answering a question
about store compliance, asks before spending one of the month's audits, and
does not tell somebody their app is compliant. An agent that has not read it
guesses at all three.
