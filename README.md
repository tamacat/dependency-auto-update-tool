# dependency-auto-update-tool

A standalone, on-demand CLI tool for a single-module Maven project: it
detects candidate dependency updates, judges compatibility by actually
running `mvn clean verify` against each candidate, and applies
validated-safe updates to `pom.xml` with a configurable commit/push mode.

This tool is general-purpose -- it makes no assumption about which project
it runs against beyond "a git-managed, single-module Maven project." Which
branches it is allowed to operate on, and what JDK each one expects, is a
policy you declare in `config/tool.properties` (see
[Configuration](#configuration)); the tool ships no hardcoded branch list of
its own. It was originally built for, and ships pre-configured for, its
reference deployment: [`tamacat-httpd`](https://github.com/tamacat/tamacat-httpd)
(branches `v1.6` -> JDK 8, `v1.7` -> JDK 25, `v2.0` -> JDK 25).

## What it does, in order

1. **Pre-flight**: detects the currently checked-out branch, confirms it has
   a configured entry in `config/tool.properties` (see
   [Configuration](#configuration)), resolves that branch's configured JDK
   home (see [Setup](#setup) below), confirms it reports the expected major
   version, then confirms the working tree is clean. Any failure stops the
   run immediately, before any dependency is even looked at.
2. **Detection**: parses `pom.xml`'s direct, non-test-scope dependencies
   (excluding any dependency that shares the target project's own
   `groupId` -- a sibling artifact of the same project, not a third-party
   library), groups any that share a single `${property}` version reference
   (e.g. tamacat-httpd's own `tomcat.version`, shared by `tomcat-embed-core`
   and `tomcat-embed-jasper`) into one atomic update target, queries a Maven
   repository (Maven Central by default) for each target's available
   versions, and filters by the configured version-range policy
   (patch/minor/major, default patch-only). It also looks up known
   vulnerabilities in the current versions (see
   [Security fixes](#security-fixes)); a version that fixes them may go
   beyond the normal policy, and security fixes are validated first.
   Versions whose jars need a newer Java than the branch's JDK are left out
   before anything is chosen (see [Java compatibility](#java-compatibility)).
3. **Validation**: for each candidate, temporarily rewrites `pom.xml`, runs
   `mvn clean verify` under the branch's configured JDK, and classifies the
   result as safe (`GREEN`), genuinely incompatible (`INCOMPATIBLE`), or an
   infrastructure hiccup unrelated to the candidate (`INFRA_FAILURE`). A
   build that passes but shows a class that could not be loaded because it
   was built for a newer Java is `INCOMPATIBLE`, not `GREEN`.
4. **Applying**: a `GREEN` candidate is committed. Depending on the
   configured mode, the tool either stops there (the default) or also
   pushes.
5. **Reporting**: everything above is written to the console and to a
   structured report file.

This tool was originally designed and built alongside
[`tamacat-httpd`](https://github.com/tamacat/tamacat-httpd) and later
extracted into its own repository. The behavior that matters to users is
described in this README and in the comments of `config/tool.properties`.
Some source code comments refer to the original design notes by identifier
(for example `BR-3` for a business rule or `ADR-4` for an architecture
decision); those notes are not published, and the comments themselves
describe the behavior they refer to.

## Building

Building and running this tool requires JDK 25 or later
(`maven.compiler.release` is 25). This is the JDK this tool itself runs on,
independent of the JDKs it validates each target branch with (see
[Setup](#setup-once-per-machine-before-first-use)).

```
mvn clean package
```

Produces `target/dependency-auto-update-tool-1.2.0.jar` (a plain jar with a
`Main-Class` manifest entry -- this tool has no runtime dependencies beyond
the JDK, so no shading/fat-jar step is needed).

## Setup (once per machine, before first use)

The tool validates dependency updates by actually building the target
project under **that branch's own JDK**, which is deliberately independent
of whatever JDK this tool's own code happens to run on. You must tell it
where each configured branch's JDK lives on your machine:

```
cp config/jdk-home.local.properties.template config/jdk-home.local.properties
```

Then edit `config/jdk-home.local.properties` and set real paths for each
branch declared in `config/tool.properties`, e.g. (tamacat-httpd's own
reference values):

```
jdkHome.v1.6=/opt/amazon-corretto-8
jdkHome.v1.7=/opt/amazon-corretto-25
jdkHome.v2.0=/opt/amazon-corretto-25
```

**This file is deliberately not committed to git** (see `.gitignore`) -- a
JDK installation path is a fact about one machine, not a repo-wide policy
every clone should share. If you skip this step, point it at the wrong JDK,
or check out a branch with no entry in `config/tool.properties` at all, the
tool fails fast at pre-flight with a clear `BRANCH_JDK_MISMATCH` error
rather than silently validating under the wrong Java version.

## Configuration

Two independent, `.properties`-format config files (chosen at Code
Generation -- no new dependency for YAML parsing):

| File | Committed? | Holds |
|---|---|---|
| `config/tool.properties` | Yes | Commit/push mode; which branches this tool may operate on and each one's expected JDK major version; per-dependency version-range policy; repository URL/timeout/report-dir overrides |
| `config/jdk-home.local.properties` | **No** (gitignored) | Per-branch JDK home paths -- machine-local, see Setup above |

`config/tool.properties` ships pre-configured for this tool's reference
deployment (tamacat-httpd's two branches, safe commit-mode/policy defaults)
-- see the file's own comments for every key and its exact syntax. **Adopting
this tool for a different project**: edit the `branch.<name>.expectedJdkMajor`
entries to declare that project's own branches and their JDK targets; a
branch with no entry is refused at pre-flight rather than silently
validated. Note that a `policy.<groupId>:<artifactId>` key's colon **must**
be escaped as `\:` in the file -- `java.util.Properties` treats an unescaped
`:` as a key/value separator, same as `=` (see the file's own comment for
why, and `ToolConfigurationTest` for the regression test).

## Running

```
java -jar target/dependency-auto-update-tool-1.2.0.jar <repoRoot>
```

The one required argument is the path to the target project's checkout to
operate on (its `pom.xml` must sit directly at `<repoRoot>/pom.xml`, i.e. a
single-module project or a specific module directory -- this tool does not
walk a multi-module reactor). Two optional arguments override the config
file paths (both otherwise resolved relative to the current working
directory):

```
java -jar target/dependency-auto-update-tool-1.2.0.jar <repoRoot> [toolConfigPath] [jdkHomeConfigPath]
```

The report file lands under `reports/` (relative to the current working
directory) -- gitignored, one file per run.

## Running on a schedule (GitHub Actions)

The tool itself has no scheduler, but this repository ships a reusable
workflow, [`.github/workflows/dependency-auto-update.yml`](.github/workflows/dependency-auto-update.yml),
that runs it on a schedule from the target project's own repository and
turns each branch's validated updates into a pull request for review:

1. **setup** reads `branch.<name>.expectedJdkMajor` from the target
   repository's `tool.properties` (default
   `.github/dependency-auto-update/tool.properties` on its default branch)
   and runs one job per branch.
2. **update** checks out the branch, installs that branch's JDK plus JDK 25
   for the tool, downloads the tool's jar from this repository's release
   (verified with `gh attestation verify` against `release.yml` and the
   tag), and runs it with `commitPushMode` forced to
   `LOCAL_COMMIT_MANUAL_APPROVAL`. The new commits are saved as patches.
   This job runs every candidate's `mvn clean verify`, so it has read-only
   permission and no stored git credentials. A non-zero exit code fails
   this job, so you get a notification when something needs attention.
3. **pr** applies the patches without running any build, refuses them if
   they change anything other than `pom.xml`, force-pushes them to
   `dependency-auto-update/<branch>`, and opens a pull request against the
   branch (or updates the open one).

Nothing is merged automatically: the review step before merging is what
catches a compromised but compatible release, as described in
[Commit/push modes](#commitpush-modes).

To set it up in a target repository:

1. Copy [`examples/github-actions/dependency-auto-update.yml`](examples/github-actions/dependency-auto-update.yml)
   to its `.github/workflows/`. Replace `<commit-sha>` with the commit of
   this repository to use, and set `tool-version` to a release tag.
2. Put a `tool.properties` at `.github/dependency-auto-update/tool.properties`
   on its default branch (start from `config/tool.properties`).
   `config/jdk-home.local.properties` is not needed: the workflow writes it.
3. Under Settings > Actions > General > Workflow permissions, turn on
   "Allow GitHub Actions to create and approve pull requests".

Each run rebuilds the pull request branch from the target branch's latest
commit, so commits added to that branch by hand are lost. A pull request
made with `GITHUB_TOKEN` does not trigger other workflows (such as CodeQL)
by itself; close and reopen it to run them.

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

## Security fixes

Unless `securityFixPolicy=OFF`, Detection asks [OSV.dev](https://osv.dev/)
(`api.osv.dev`) which known vulnerabilities (GHSA, CVE, ...) affect each
target's current version and its newer versions. Only Maven package URLs
(groupId, artifactId, version) are sent -- one batched request per target,
using the JDK's own HTTP client. For a `${property}` group, every member is
checked.

- **Not vulnerable**: nothing changes; the normal policy picks the candidate.
- **Vulnerable, and the normal candidate fixes it**: same candidate, marked as
  a security fix.
- **Vulnerable, and the normal candidate does not fix it**: the smallest
  series (major.minor) that fixes every known vulnerability is found, and its
  newest fixing version becomes the candidate -- up to the bump size set by
  `securityFixPolicy` (default `MINOR`; `PATCH_ONLY`, `MINOR`, `MAJOR`, or
  `OFF`). Only stable versions are considered, unless the current version is
  itself a pre-release. For example, log4j-core 2.14.1 under `PATCH_ONLY`:
  no 2.14.x fixes Log4Shell, so 2.25.5 (the newest of 2.25, the smallest
  series that fixes all eight advisories) is proposed.
- **Nothing allowed fixes everything**: the normal candidate is kept and the
  report lists what it fixes and what remains; with no candidate at all, the
  target is reported as `NO_UPDATE_AVAILABLE` with its vulnerabilities.

A larger-than-usual bump is still a normal candidate: it must pass
`mvn clean verify`, and in the default mode it is only committed locally for
review. Security fixes are validated and committed **before** other
candidates, so a commit failure later in the run cannot hold them back.

In the report, such lines carry `vulnerable="..."`, `fixes="..."`,
`remaining="..."` and `securityEscalated=true` (for a bump beyond the normal
policy). A failed lookup does not stop the run: the target is handled as
before, with `securityDiagnostic="..."`. The commit message becomes
`fix(deps): bump ... (Fixes known vulnerabilities: GHSA-...)` instead of
`chore(deps): ...`.

## Java compatibility

A version built for a newer Java than the branch's JDK is never proposed,
including as a security fix. Passing `mvn clean verify` does not prove that
a version runs: a library that is loaded only at run time can fail to load
while every test still passes. For example, logback-core 1.5 (built for
Java 11) on a JDK 8 branch: SLF4J cannot load logback, falls back to a
logger that drops every message, and the build still succeeds.

- **Before choosing**: Detection downloads the jars of newer versions from
  the same Maven repository and reads the class-file version of every class
  in them. `module-info.class` and `META-INF/versions/` (classes that an
  older JVM ignores) are skipped. Versions that need a newer Java than
  `branch.<name>.expectedJdkMajor` are left out. Only versions that the
  dependency's policy or the security-fix ceiling could choose are checked.
  The Java a library needs is assumed not to go down as its version goes
  up, so the cut-off is found by binary search: usually only the newest jar
  is read. For a `${property}` group, a version is left out if any member
  needs a newer Java. A jar that cannot be downloaded makes the target
  `QUERY_FAILED`. A version without a jar is not left out.
- **After building**: a build that exits with 0 but whose output shows
  `UnsupportedClassVersionError`, "has been compiled by a more recent
  version of the Java Runtime", or SLF4J's "A service provider failed to
  instantiate" is classified as `INCOMPATIBLE`.

In the report, a target with left-out versions carries
`javaExcluded="1.4.0 and newer need Java 11 (branch JDK 8)"`. If those were
the only versions that fix a known vulnerability, the target is reported as
`NO_UPDATE_AVAILABLE` with the vulnerabilities under `remaining`. Fixing
them then needs a decision the tool cannot make, such as moving the branch
to a newer Java.

## Exit code contract

- `0`: every target either had nothing new to apply, or was successfully
  applied.
- Non-zero (`1`): the run stopped at pre-flight, or at least one target
  needs attention -- a repository query failed, a candidate was
  incompatible or hit an infra failure during validation, a commit failed,
  or a commit succeeded but the subsequent push failed.
- `2`: usage error (missing required argument, unreadable/invalid config file).

## What this tool deliberately does not do

- No scheduling/cron trigger in the tool itself -- it is on-demand logic
  only; run it however and whenever you like (cron, a manual invocation, a
  CI job). For GitHub, see
  [Running on a schedule](#running-on-a-schedule-github-actions).
- No transitive-dependency scanning -- only the target project's own direct,
  non-test-scope `pom.xml` dependencies are considered.
- No vulnerability-free guarantee -- known vulnerabilities only steer which
  candidate is proposed (see [Security fixes](#security-fixes)). Whether an
  update is safe to take is still judged by "does the real build/test suite
  still pass," and an update can only fix what is already known.
- No multi-module reactor support -- it reads exactly one `pom.xml` at the
  given `repoRoot`.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).

The command-line tool has no runtime dependencies beyond the JDK. The
[`software-update-checker`](software-update-checker/) GUI bundles third-party
open source components; their licenses are listed in
[`software-update-checker/THIRD-PARTY-NOTICES.txt`](software-update-checker/THIRD-PARTY-NOTICES.txt).
