# dependency-auto-update-tool

A standalone, on-demand CLI tool that detects candidate Maven dependency
updates for `tamacat-httpd`, judges compatibility by actually running
`mvn clean verify` against each candidate, and applies validated-safe
updates to `pom.xml` with a configurable commit/push mode.

This module is **intentionally not part of the main `tamacat-httpd` build**
-- there is no `<modules>` entry for it in the root `pom.xml`, and it is
built/run independently.

## What it does, in order

1. **Pre-flight**: detects the currently checked-out branch, resolves that
   branch's configured JDK home (see [Setup](#setup) below) and confirms it
   reports the expected major version (`master` -> 8, `v2.0-tc11` -> 25),
   then confirms the working tree is clean. Either failure stops the run
   immediately, before any dependency is even looked at.
2. **Detection**: parses `pom.xml`'s direct, non-test-scope dependencies,
   groups any that share a single `${property}` version reference (e.g. this
   project's own `tomcat.version`, shared by `tomcat-embed-core` and
   `tomcat-embed-jasper`) into one atomic update target, queries Maven
   Central for each target's available versions, and filters by the
   configured version-range policy (patch/minor/major, default patch-only).
3. **Validation**: for each candidate, temporarily rewrites `pom.xml`, runs
   `mvn clean verify` under the branch's configured JDK, and classifies the
   result as safe (`GREEN`), genuinely incompatible (`INCOMPATIBLE`), or an
   infrastructure hiccup unrelated to the candidate (`INFRA_FAILURE`).
4. **Applying**: a `GREEN` candidate is committed. Depending on the
   configured mode, the tool either stops there (the default) or also
   pushes.
5. **Reporting**: everything above is written to the console and to a
   structured report file.

See `../../aidlc/spaces/default/intents/260903-dependency-auto-update/` in
this project's AI-DLC record for the full design rationale (requirements,
architecture decisions, business rules).

## Building

```
mvn -f tools/dependency-auto-update/pom.xml clean package
```

Produces `target/dependency-auto-update-tool-1.0.0.jar` (a plain jar with a
`Main-Class` manifest entry -- this tool has no runtime dependencies beyond
the JDK, so no shading/fat-jar step is needed).

## Setup (once per machine, before first use)

The tool validates dependency updates by actually building `tamacat-httpd`
under **that branch's own JDK** -- `master` targets Java 8, `v2.0-tc11`
targets Java 25 -- which is deliberately independent of whatever JDK this
tool's own code happens to run on. You must tell it where each branch's JDK
lives on your machine:

```
cd tools/dependency-auto-update
cp config/jdk-home.local.properties.template config/jdk-home.local.properties
```

Then edit `config/jdk-home.local.properties` and set real paths, e.g.:

```
jdkHome.master=/opt/amazon-corretto-8
jdkHome.v2.0-tc11=/opt/amazon-corretto-25
```

**This file is deliberately not committed to git** (see `.gitignore`) -- a
JDK installation path is a fact about one machine, not a repo-wide policy
every clone should share. If you skip this step, or point it at the wrong
JDK, the tool fails fast at pre-flight with a clear `BRANCH_JDK_MISMATCH`
error rather than silently validating under the wrong Java version.

## Configuration

Two independent, `.properties`-format config files (chosen at Code
Generation -- no new dependency for YAML parsing):

| File | Committed? | Holds |
|---|---|---|
| `config/tool.properties` | Yes | Commit/push mode; per-dependency version-range policy; repository URL/timeout/report-dir overrides |
| `config/jdk-home.local.properties` | **No** (gitignored) | Per-branch JDK home paths -- machine-local, see Setup above |

`config/tool.properties` ships with safe defaults (local-commit,
patch-only) -- see the file's own comments for every key and an example.

## Running

```
cd tools/dependency-auto-update
java -jar target/dependency-auto-update-tool-1.0.0.jar ../..
```

The one required argument is the path to the `tamacat-httpd` checkout to
operate on (`../..` from this module's own directory, i.e. the submodule
root that contains `pom.xml`). Two optional arguments override the config
file paths (both otherwise resolved relative to the current working
directory):

```
java -jar target/dependency-auto-update-tool-1.0.0.jar <repoRoot> [toolConfigPath] [jdkHomeConfigPath]
```

The report file lands under `reports/` (relative to the current working
directory, i.e. `tools/dependency-auto-update/reports/` when invoked as
shown above) -- gitignored, one file per run.

## Commit/push modes

- **`LOCAL_COMMIT_MANUAL_APPROVAL`** (default): a `GREEN` candidate is
  committed locally; the maintainer reviews and pushes manually. This is the
  primary mitigation against this tool's central, disclosed risk: a
  compromised-but-functionally-compatible dependency release would still
  pass `mvn clean verify` and be classified `GREEN` -- the manual-review
  step before `origin` is what catches what the build cannot.
- **`FULL_LOOP_AUTO_PUSH`** (opt-in): also pushes automatically once the
  commit succeeds. Materially increases supply-chain exposure by removing
  that review step -- opt in deliberately.

Whichever mode is configured, a **failed commit stops the run** (no further
candidates are attempted this invocation) so that a later candidate's
successful commit can never silently bundle an earlier, still-uncommitted
change. If you see a run fail pre-flight with `DIRTY_WORKING_TREE`, check
`git status`/`git diff` first -- it is very likely an earlier run's
unresolved commit failure, not a new problem.

## Exit code contract

- `0`: every target either had nothing new to apply, or was successfully
  applied.
- Non-zero (`1`): the run stopped at pre-flight, or at least one target
  needs attention -- a repository query failed, a candidate was
  incompatible or hit an infra failure during validation, a commit failed,
  or a commit succeeded but the subsequent push failed.
- `2`: usage error (missing required argument, unreadable/invalid config file).

## What this tool deliberately does not do

- No scheduling/cron trigger -- this is on-demand logic only; run it however
  and whenever you like (cron, a manual invocation, a future CI job).
- No transitive-dependency scanning -- only this project's own direct,
  non-test-scope `pom.xml` dependencies are considered.
- No vulnerability-database integration -- this tool's compatibility
  judgment is "does the real build/test suite still pass," not "is this
  version free of known CVEs."
- Does not read, modify, or depend on the existing `versions-maven-plugin`
  configuration in the main `pom.xml` -- that plugin stays exactly as it is,
  display-only.
